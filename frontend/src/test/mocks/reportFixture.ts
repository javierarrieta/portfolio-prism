import type { TaxReport } from '../../types'

/**
 * One payload, shared by the component test, the App test and the MSW branch, so all three
 * describe the same report.
 *
 * Totals that deliberately disagree with the rows: `lot_pnl` sums to 1800.00 while
 * `realized_in_window` says 1234.56. A renderer that adds rows up shows 1800.00 and the
 * test fails. The money is strings on purpose, exactly as the server sends it -- the report
 * is the one payload that is not normalized through `number` (types.ts), so a renderer that
 * tried to add these would concatenate them and be visible immediately.
 */
export const REPORT_FIXTURE: TaxReport = {
  context: {
    portfolio_id: 1, portfolio_name: 'Retirement', base_currency: 'USD', strategy: 'FIFO',
    threshold_days: 30, requested_from: '2024-01-01', requested_to: '2024-06-30',
    splits_note: 'Splits are as recorded.',
  },
  asset_totals: [{
    asset_id: 1, symbol: 'AAPL', asset_type: 'STOCK', asset_currency: 'USD',
    realized_in_window: '1234.56', realized_cumulative: '9999.99',
    unrealized_at_end: '2500.00', market_value: '12500.00', cost_basis: '10000.00',
    price_date_used: '2024-06-28', fx_rate_date_used: null, price_source: 'database',
    fx_applied: false, split_applied: false,
  }],
  totals: {
    realized_in_window: '1234.56', realized_cumulative: '9999.99',
    unrealized_at_end: '2500.00', market_value: '12500.00', cost_basis: '10000.00',
  },
  rows: [
    {
      row_type: 'SOLD_LOT', asset_id: 1, symbol: 'AAPL', asset_type: 'STOCK', asset_currency: 'USD',
      price_date_used: '2024-06-28', fx_rate_date_used: null, price_source: 'database',
      sell_transaction_id: 3, sell_date: '2024-05-01T00:00:00Z', sold_qty: '80.0',
      sell_price: '130.0', sell_fee: '0.0', proceeds: '7800.0', cost_basis: '6000.0',
      realized_pnl: '1800.0', matched_qty: '60.0', unmatched_qty: '20.0',
      buy_transaction_id: 1, buy_date: '2024-01-01T00:00:00Z', buy_price: '100.0',
      unit_cost: '100.0', lot_matched_qty: '60.0', lot_qty_before_match: '60.0',
      lot_cost_basis: '6000.0', lot_proceeds: '7800.0', lot_pnl: '1800.0',
      holding_days: 121, term: null, original_qty: null, remaining_qty: null,
      market_value: null, unrealized_gain_loss: null, unrealized_roi: null,
    },
  ],
}
