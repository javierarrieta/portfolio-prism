import type {
  Asset, HistoryItem, LotMatch, PerformanceMetrics, PortfolioPerformance,
  SellPnlBreakdown, TaxSummary, Transaction,
} from '../types';

export type DecimalLike = string | number | null | undefined;

type UnknownRecord = Record<string, unknown>;

const asRecord = (v: unknown): UnknownRecord =>
  typeof v === 'object' && v !== null ? (v as UnknownRecord) : {};

const str = (r: UnknownRecord, key: string): string =>
  typeof r[key] === 'string' ? (r[key] as string) : '';

const num = (r: UnknownRecord, key: string): number =>
  toNumber(r[key] as DecimalLike);

export const toNumber = (v: DecimalLike): number => {
  if (v === null || v === undefined || v === '') return 0;
  const n = typeof v === 'number' ? v : Number(v);
  return Number.isFinite(n) ? n : 0;
};

export const normalizeHistory = (h: unknown): HistoryItem => {
  const r = asRecord(h);
  return {
    date: str(r, 'date'),
    value: num(r, 'value'),
    daily_return: num(r, 'daily_return'),
    twr: num(r, 'twr'),
    cash_flow: num(r, 'cash_flow'),
  };
};

export const normalizeMetrics = (m: unknown): PerformanceMetrics => {
  const r = asRecord(m);
  return {
    volatility: num(r, 'volatility'),
    sharpe_ratio: num(r, 'sharpe_ratio'),
    beta: num(r, 'beta'),
    portfolio_value: num(r, 'portfolio_value'),
    beta_adjusted_exposure: num(r, 'beta_adjusted_exposure'),
    unrealized_pnl: r.unrealized_pnl !== undefined ? toNumber(r.unrealized_pnl as DecimalLike) : undefined,
    realized_pnl: r.realized_pnl !== undefined ? toNumber(r.realized_pnl as DecimalLike) : undefined,
  };
};

export const normalizePerformance = (p: unknown): PortfolioPerformance => {
  const r = asRecord(p);
  const history = Array.isArray(r.history) ? (r.history as unknown[]) : [];
  return {
    history: history.map(normalizeHistory),
    correlation_matrix: (r.correlation_matrix as Record<string, Record<string, number>>) ?? {},
    metrics: normalizeMetrics(r.metrics),
  };
};

export const normalizeTaxSummary = (t: unknown): TaxSummary => {
  const r = asRecord(t);
  const assets = Array.isArray(r.assets) ? (r.assets as unknown[]) : [];
  return {
    currency: str(r, 'currency') || 'USD',
    total_portfolio_value: num(r, 'total_portfolio_value'),
    total_realized_pnl: num(r, 'total_realized_pnl'),
    total_unrealized_pnl: num(r, 'total_unrealized_pnl'),
    assets: assets.map((a: unknown) => {
      const ar = asRecord(a);
      const tax_lots = Array.isArray(ar.tax_lots) ? (ar.tax_lots as unknown[]) : [];
      return {
        symbol: str(ar, 'symbol'), asset_type: str(ar, 'asset_type'),
        current_shares: num(ar, 'current_shares'), average_cost: num(ar, 'average_cost'),
        current_price: num(ar, 'current_price'), total_cost: num(ar, 'total_cost'),
        market_value: num(ar, 'market_value'), unrealized_pnl: num(ar, 'unrealized_pnl'),
        unrealized_roi: num(ar, 'unrealized_roi'), realized_pnl: num(ar, 'realized_pnl'),
        tax_lots: tax_lots.map((l: unknown) => {
          const lr = asRecord(l);
          return {
            buy_date: str(lr, 'buy_date'),
            buy_price: num(lr, 'buy_price'), original_qty: num(lr, 'original_qty'),
            remaining_qty: num(lr, 'remaining_qty'), latent_gain_loss: num(lr, 'latent_gain_loss'),
            latent_roi: num(lr, 'latent_roi'),
          };
        }),
      };
    }),
  };
};

export const normalizeTransactionList = (txs: unknown[]): Transaction[] => txs.map((t: unknown) => {
  const r = asRecord(t);
  return {
    id: r.id as number, asset_id: r.asset_id as number, type: r.type as Transaction['type'], date: str(r, 'date'),
    quantity: num(r, 'quantity'),
    price: num(r, 'price'),
    fee: num(r, 'fee'),
  };
});

export const normalizeAsset = (a: unknown): Asset => {
  const r = asRecord(a);
  const sector = r.sector;
  const transactions = Array.isArray(r.transactions) ? (r.transactions as unknown[]) : [];
  return {
    id: r.id as number,
    portfolio_id: r.portfolio_id as number,
    symbol: str(r, 'symbol'),
    name: str(r, 'name'),
    asset_type: r.asset_type as Asset['asset_type'],
    sector: typeof sector === 'string' ? sector : undefined,
    currency: str(r, 'currency'),
    isin: r.isin === null || r.isin === undefined ? null : String(r.isin),
    transactions: normalizeTransactionList(transactions),
  };
};

export const normalizeAssetList = (assets: unknown[]): Asset[] => assets.map(normalizeAsset);
const optNum = (r: UnknownRecord, key: string): number | null =>
  r[key] === null || r[key] === undefined ? null : toNumber(r[key] as DecimalLike);

// Only the two literals the engine emits survive. Anything else becomes null
// rather than being passed through as a tax term the backend never asserted.
const termOf = (v: unknown): 'SHORT' | 'LONG' | null =>
  v === 'SHORT' || v === 'LONG' ? v : null;

export const normalizeSellPnl = (t: unknown): SellPnlBreakdown => {
  const r = asRecord(t);
  const matches = Array.isArray(r.matches) ? (r.matches as unknown[]) : [];
  return {
    // num(), not `as number`: a cast is compile-time only, so a payload carrying
    // "transaction_id":"42" would hand the UI a string typed as number.
    transaction_id: num(r, 'transaction_id'),
    asset_id: num(r, 'asset_id'),
    symbol: str(r, 'symbol'),
    asset_type: str(r, 'asset_type'),
    strategy: str(r, 'strategy'),
    threshold_days: num(r, 'threshold_days'),
    currency: str(r, 'currency') || 'USD',
    asset_currency: str(r, 'asset_currency') || 'USD',
    fx_applied: r.fx_applied === true,
    split_applied: r.split_applied === true,
    sell_date: str(r, 'sell_date'),
    sold_qty: num(r, 'sold_qty'),
    price: num(r, 'price'),
    fee: num(r, 'fee'),
    proceeds: num(r, 'proceeds'),
    cost_basis: num(r, 'cost_basis'),
    realized_pnl: num(r, 'realized_pnl'),
    roi: optNum(r, 'roi'),
    matched_qty: num(r, 'matched_qty'),
    unmatched_qty: num(r, 'unmatched_qty'),
    // Left in payload order: that is the engine's consumption order, and sorting
    // here would hide it from the tests that assert on it.
    matches: matches.map((m: unknown): LotMatch => {
      const mr = asRecord(m);
      return {
        buy_transaction_id: (mr.buy_transaction_id as number) ?? 0,
        buy_date: str(mr, 'buy_date'),
        buy_price: num(mr, 'buy_price'),
        unit_cost: num(mr, 'unit_cost'),
        matched_qty: num(mr, 'matched_qty'),
        lot_qty_before_match: num(mr, 'lot_qty_before_match'),
        cost_basis: num(mr, 'cost_basis'),
        proceeds: num(mr, 'proceeds'),
        pnl: num(mr, 'pnl'),
        holding_days: num(mr, 'holding_days'),
        term: termOf(mr.term),
      };
    }),
  };
};
