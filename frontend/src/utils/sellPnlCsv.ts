import Papa from 'papaparse';
import type { LotMatch, SellPnlBreakdown } from '../types';

/**
 * Serialize one sell's lot attribution for a spreadsheet or tax tool.
 *
 * Deliberately built from the normalized numbers, never from `utils/formatters`:
 * `formatCurrency` emits `$1,234.56` (the commas shift columns) and `formatPercent`
 * emits `+23.81%`. Full precision is the standing decision for this feature -- see
 * docs/superpowers/reviews/2026-11-05-sell-pnl-breakdown.md, which rejects per-field
 * rounding as actively harmful because a rounded sum no longer equals the total.
 *
 * Shape: one row per lot with the sell-level context repeated on every row, so the file
 * pivots. Caveats ride along as columns rather than a leading `#` row, which breaks naive
 * CSV readers. Nulls are blank cells, not `n/a`, so a numeric column stays numeric.
 */

// Sell-level names are the API's, verbatim, with one deliberate exception: price and fee
// are the two columns in asset_currency rather than the base currency, so they say so in
// their own header. Lot-level names that would collide with a sell-level one (matched_qty,
// cost_basis, proceeds, pnl) are prefixed lot_; the rest keep their payload names.
// `realized_pnl` and `lot_pnl` therefore never get confused for each other.
const SELL_COLUMNS = [
  'transaction_id', 'asset_id', 'symbol', 'asset_type', 'strategy', 'threshold_days',
  'currency', 'asset_currency', 'fx_applied', 'split_applied', 'sell_date', 'sold_qty',
  'price_in_asset_currency', 'fee_in_asset_currency', 'proceeds', 'cost_basis', 'realized_pnl',
  'roi', 'matched_qty', 'unmatched_qty',
] as const;

const LOT_COLUMNS = [
  'buy_date', 'unit_cost', 'lot_matched_qty', 'lot_qty_before_match', 'lot_cost_basis',
  'lot_proceeds', 'lot_pnl', 'holding_days', 'term',
] as const;

const CSV_COLUMNS = ['row_type', ...SELL_COLUMNS, ...LOT_COLUMNS] as const;

type Cell = string | number | boolean | null;
type Row = Record<string, Cell>;

// Papa.unparse renders null as an empty cell, which is the chosen null token.
const sellCells = (d: SellPnlBreakdown): Row => ({
  transaction_id: d.transaction_id,
  asset_id: d.asset_id,
  symbol: d.symbol,
  asset_type: d.asset_type,
  strategy: d.strategy,
  threshold_days: d.threshold_days,
  currency: d.currency,
  asset_currency: d.asset_currency,
  fx_applied: d.fx_applied,
  split_applied: d.split_applied,
  sell_date: d.sell_date,
  sold_qty: d.sold_qty,
  // As entered, in asset_currency -- the same convention the API and the modal footnote use.
  price_in_asset_currency: d.price,
  fee_in_asset_currency: d.fee,
  proceeds: d.proceeds,
  cost_basis: d.cost_basis,
  realized_pnl: d.realized_pnl,
  roi: d.roi,
  matched_qty: d.matched_qty,
  unmatched_qty: d.unmatched_qty,
});

const lotCells = (m: LotMatch): Row => ({
  buy_date: m.buy_date,
  unit_cost: m.unit_cost,
  lot_matched_qty: m.matched_qty,
  lot_qty_before_match: m.lot_qty_before_match,
  lot_cost_basis: m.cost_basis,
  lot_proceeds: m.proceeds,
  lot_pnl: m.pnl,
  holding_days: m.holding_days,
  term: m.term,
});

// Unmatched quantity carries no cost basis and contributes nothing to realized_pnl, so it
// gets a row of its own with the money cells left blank rather than zeroed: a spreadsheet
// SUM skips blanks, so the pnl column keeps totalling exactly realized_pnl. Putting its
// quantity in lot_matched_qty is what makes the other whole-column invariant hold --
// summing that column accounts for every sold unit.
const unmatchedCells = (d: SellPnlBreakdown): Row => ({
  lot_matched_qty: d.unmatched_qty,
  buy_date: '',
  unit_cost: '',
  lot_qty_before_match: '',
  lot_cost_basis: '',
  lot_proceeds: '',
  lot_pnl: '',
  holding_days: '',
  term: '',
});

export const sellPnlToCsv = (data: SellPnlBreakdown): string => {
  const rows: Row[] = data.matches.map((m) => ({
    row_type: 'LOT',
    ...sellCells(data),
    ...lotCells(m),
  }));
  if (data.unmatched_qty > 0) {
    rows.push({ row_type: 'UNMATCHED', ...sellCells(data), ...unmatchedCells(data) });
  }
  // CRLF is Papa's default and RFC 4180's requirement; Papa does the quoting, so a symbol
  // with a comma in it cannot shift columns.
  return Papa.unparse({ fields: [...CSV_COLUMNS], data: rows });
};

// A filename date taken through `new Date(...)` shifts a day for any reader whose local
// zone is offset from the payload's, so this slices the payload string instead.
const DATE_PREFIX = /^\d{4}-\d{2}-\d{2}/;

export const sellPnlFilename = (data: SellPnlBreakdown): string => {
  // Symbols are user-entered: keep 'BRK.B', neutralise a '/' that would become a path.
  const symbol = data.symbol.replace(/[^A-Za-z0-9._-]/g, '_') || 'SELL';
  const date = DATE_PREFIX.exec(data.sell_date)?.[0] ?? 'unknown';
  return `${symbol}_${date}_sell-pnl.csv`;
};
