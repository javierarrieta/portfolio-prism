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
 *
 * The repetition has a cost worth stating rather than leaving to luck: SUM(realized_pnl)
 * over the rows is that total multiplied by the number of rows, and the same goes for
 * proceeds, cost_basis and sold_qty. The column that totals is lot_pnl -- that is what
 * row_type is there to separate LOT from UNMATCHED. A consumer wanting sell-level figures
 * takes them from one row per transaction_id, it does not sum them.
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
  'buy_date', 'buy_price', 'unit_cost', 'lot_matched_qty', 'lot_qty_before_match',
  'lot_cost_basis', 'lot_proceeds', 'lot_pnl', 'holding_days', 'term',
] as const;

const CSV_COLUMNS = ['row_type', ...SELL_COLUMNS, ...LOT_COLUMNS] as const;

type Cell = string | number | boolean | null;
type Row = Record<string, Cell>;

// Excel and Google Sheets run a cell whose first character is `=`, `+`, `-`, `@` or a tab,
// and a symbol reaches the file as free text: csvValidator only checks it is non-empty and
// the backend stores it uppercased and unfiltered, so a symbol imported from somebody
// else's CSV is attacker-chosen by the time it is exported. The leading apostrophe is what
// makes a spreadsheet render the cell as text instead of running it. Applied to the four
// columns a user can type into at asset creation; every other cell is either a number
// written by `num` or a literal the engine emitted. The apostrophe does show up in the
// imported cell -- that is the cost of a guard which cannot corrupt a legitimate value,
// since no real symbol, asset type or currency code starts with one of those characters.
const FORMULA_LEAD = /^[=+\-@\t\r]/;
const text = (v: string): string => (FORMULA_LEAD.test(v) ? `'${v}` : v);

// String(1e-7) is '1e-7', and a money column that needs exponent notation is a tax tool
// waiting to misread it. Fall back to fixed notation when the default repr uses an
// exponent. Two ranges that fallback cannot help: below 1e-20 it collapses to 0 and above
// 1e21 toFixed itself goes exponential -- both are far outside any holding worth exporting.
const num = (v: number): string => {
  const s = String(v);
  if (!s.includes('e')) return s;
  return v.toFixed(20).replace(/0+$/, '').replace(/\.$/, '');
};

// Papa.unparse renders null as an empty cell, which is the chosen null token.
const sellCells = (d: SellPnlBreakdown): Row => ({
  transaction_id: num(d.transaction_id),
  asset_id: num(d.asset_id),
  symbol: text(d.symbol),
  asset_type: text(d.asset_type),
  strategy: d.strategy,
  threshold_days: num(d.threshold_days),
  currency: text(d.currency),
  asset_currency: text(d.asset_currency),
  fx_applied: d.fx_applied,
  split_applied: d.split_applied,
  sell_date: d.sell_date,
  sold_qty: num(d.sold_qty),
  // As entered, in asset_currency -- the same convention the API and the modal footnote use.
  price_in_asset_currency: num(d.price),
  fee_in_asset_currency: num(d.fee),
  proceeds: num(d.proceeds),
  cost_basis: num(d.cost_basis),
  realized_pnl: num(d.realized_pnl),
  roi: d.roi === null ? null : num(d.roi),
  matched_qty: num(d.matched_qty),
  unmatched_qty: num(d.unmatched_qty),
});

const lotCells = (m: LotMatch): Row => ({
  buy_date: m.buy_date,
  // Both, not one of them: buy_price is the split-adjusted price the modal's Buy price
  // column shows, while unit_cost additionally carries the amortised buy fee -- see the
  // LotMatch doc comment in backend_rust/src/schemas.rs. With any buy fee they differ, and
  // a reader needs buy_price to reconcile this file against the screen it came from.
  buy_price: num(m.buy_price),
  unit_cost: num(m.unit_cost),
  lot_matched_qty: num(m.matched_qty),
  lot_qty_before_match: num(m.lot_qty_before_match),
  lot_cost_basis: num(m.cost_basis),
  lot_proceeds: num(m.proceeds),
  lot_pnl: num(m.pnl),
  holding_days: num(m.holding_days),
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
