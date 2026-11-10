/**
 * Contract: decimal values (quantity, price, fee, market value, pnl, ...) arrive
 * from the API as JSON *strings* (exact Decimal transport, see backend
 * `decimal_json`). In memory the frontend stores them as *numbers*; every fetch
 * response is normalized at the boundary in `src/utils/decimal.ts`, so components
 * and these types always operate on numbers. Sending a create/update payload may
 * use either string or number for decimal fields.
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
