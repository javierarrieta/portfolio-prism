/**
 * Contract: decimal values (quantity, price, fee, market value, pnl, ...) arrive
 * from the API as JSON *strings* (exact Decimal transport, see backend
 * `decimal_json`). In memory the frontend stores them as *numbers*; every fetch
 * response is normalized at the boundary in `src/utils/decimal.ts`, so components
 * and these types always operate on numbers. Sending a create/update payload may
 * use either string or number for decimal fields.
 *
 * The dated tax report (`TaxReport`) is the one exception and says so at its definition:
 * its money stays in the exact decimal strings the server sent.
 */
export type AssetType = 'STOCK' | 'CRYPTO' | 'ETF' | 'MUTUAL_FUND';
export type TransactionType = 'BUY' | 'SELL' | 'SPLIT';

export interface Transaction {
  id: number;
  asset_id: number;
  type: TransactionType;
  quantity: number;
  price: number;
  fee: number;
  date: string;
}

export interface Asset {
  id: number;
  symbol: string;
  name: string;
  asset_type: AssetType;
  sector?: string;
  portfolio_id: number;
  currency: string;
  isin: string | null;
  transactions: Transaction[];
}

export interface Portfolio {
  id: number;
  name: string;
  description?: string;
  currency: string;
  assets: Asset[];
}

export interface TaxLot {
  buy_transaction_id: number;
  buy_date: string;
  buy_price: number;
  original_qty: number;
  remaining_qty: number;
  latent_gain_loss: number;
  latent_roi: number;
}

export interface AssetTaxSummary {
  symbol: string;
  asset_type: string;
  current_shares: number;
  average_cost: number;
  current_price: number;
  total_cost: number;
  market_value: number;
  unrealized_pnl: number;
  unrealized_roi: number;
  realized_pnl: number;
  tax_lots: TaxLot[];
}

export interface HistoryItem {
  date: string;
  value: number;
  daily_return: number;
  twr: number;
  cash_flow: number;
}

export interface PerformanceMetrics {
  volatility: number;
  sharpe_ratio: number;
  beta: number;
  portfolio_value: number;
  beta_adjusted_exposure: number;
  unrealized_pnl?: number;
  realized_pnl?: number;
}

export interface PortfolioPerformance {
  history: HistoryItem[];
  correlation_matrix: Record<string, Record<string, number>>;
  metrics: PerformanceMetrics;
}

export interface TaxSummary {
  assets: AssetTaxSummary[];
  currency: string;
  strategy?: string;
  threshold_days?: number;
  total_portfolio_value: number;
  total_realized_pnl: number;
  total_unrealized_pnl: number;
}

export interface LotMatch {
  buy_transaction_id: number;
  buy_date: string;
  buy_price: number;
  unit_cost: number;
  matched_qty: number;
  lot_qty_before_match: number;
  cost_basis: number;
  proceeds: number;
  pnl: number;
  holding_days: number;
  term: 'SHORT' | 'LONG' | null;
}

export interface SellPnlBreakdown {
  transaction_id: number;
  asset_id: number;
  symbol: string;
  asset_type: string;
  strategy: string;
  threshold_days: number;
  currency: string;
  asset_currency: string;
  fx_applied: boolean;
  /** A SPLIT on this asset dated on or before the sell: buy prices are split-adjusted. */
  split_applied: boolean;
  sell_date: string;
  sold_qty: number;
  price: number;
  fee: number;
  proceeds: number;
  cost_basis: number;
  realized_pnl: number;
  /** null when cost basis is zero; render as an em dash, never 0%. */
  roi: number | null;
  matched_qty: number;
  unmatched_qty: number;
  matches: LotMatch[];
}

/**
 * The dated tax report. Deliberately NOT normalized through `number` the way every other
 * payload in this file is (see the header): the strings the server sent are what is held in
 * React state, what is compared for staleness, and what the CSV carries.
 *
 * *** Display is a separate matter, and is quantised. *** `TotalFigure` renders through
 * `formatCurrency`, which does `Number(value)` and then `Intl` at the currency's fraction
 * digits. So a figure with more than two fractional digits -- which the engine can produce,
 * since it never rounds (`tax_engine.rs` has no `.round(`/`.set_scale(`) -- reads as a rounded
 * amount on screen and verbatim in the file. That is the same display artifact the printable
 * statement accepted (see `docs/superpowers/specs/2026-11-09-portfolio-statement-print.md`):
 * the screen is a display, the CSV is the artifact, and the CSV is authoritative. An earlier
 * revision of this comment claimed screen and file could never differ; that was false, and the
 * release note says plainly that the file is the one to reconcile against.
 */
export type ReportRowKind = 'SOLD_LOT' | 'OPEN_LOT' | 'UNMATCHED';
export type ReportPriceSource = 'database' | 'provider';

export interface ReportContext {
  portfolio_id: number;
  portfolio_name: string;
  base_currency: string;
  strategy: string;
  threshold_days: number;
  /** The window actually applied; null when the portfolio has no transaction at all. */
  requested_from: string | null;
  requested_to: string;
  splits_note: string;
}

export interface ReportAssetTotals {
  asset_id: number;
  symbol: string;
  asset_type: string;
  asset_currency: string;
  realized_in_window: string;
  realized_cumulative: string;
  unrealized_at_end: string;
  market_value: string;
  cost_basis: string;
  price_date_used: string | null;
  fx_rate_date_used: string | null;
  price_source: ReportPriceSource | null;
  fx_applied: boolean;
  split_applied: boolean;
}

export interface ReportTotals {
  realized_in_window: string;
  realized_cumulative: string;
  unrealized_at_end: string;
  market_value: string;
  cost_basis: string;
}

export interface ReportRow {
  row_type: ReportRowKind;
  asset_id: number;
  symbol: string;
  asset_type: string;
  asset_currency: string;
  price_date_used: string | null;
  fx_rate_date_used: string | null;
  price_source: ReportPriceSource | null;
  sell_transaction_id: number | null;
  sell_date: string | null;
  sold_qty: string | null;
  sell_price: string | null;
  sell_fee: string | null;
  proceeds: string | null;
  cost_basis: string | null;
  realized_pnl: string | null;
  matched_qty: string | null;
  unmatched_qty: string | null;
  buy_transaction_id: number | null;
  buy_date: string | null;
  buy_price: string | null;
  unit_cost: string | null;
  lot_matched_qty: string | null;
  lot_qty_before_match: string | null;
  lot_cost_basis: string | null;
  lot_proceeds: string | null;
  /** The only column whose sum over SOLD_LOT rows is an asset's realized_in_window. */
  lot_pnl: string | null;
  holding_days: number | null;
  term: 'SHORT' | 'LONG' | null;
  original_qty: string | null;
  remaining_qty: string | null;
  market_value: string | null;
  unrealized_gain_loss: string | null;
  unrealized_roi: string | null;
}

export interface TaxReport {
  context: ReportContext;
  asset_totals: ReportAssetTotals[];
  totals: ReportTotals;
  rows: ReportRow[];
}

export interface ReportErrorBody {
  error: string;
  missing_prices: string[];
  missing_rates: { from: string; to: string }[];
  missing_prices_total: number;
  missing_rates_total: number;
  retryable: boolean;
}
