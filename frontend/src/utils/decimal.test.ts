import { describe, it, expect } from 'vitest'
import { normalizeSellPnl, normalizeTaxSummary } from './decimal'

const raw = {
  transaction_id: 42, asset_id: 7, symbol: 'AAPL', asset_type: 'STOCK',
  strategy: 'FIFO', threshold_days: 30,
  currency: 'USD', asset_currency: 'USD', fx_applied: false, split_applied: false,
  sell_date: '2024-12-01T00:00:00Z',
  sold_qty: '80.0', price: '130.0', fee: '0.0',
  proceeds: '10400.0', cost_basis: '8400.0', realized_pnl: '2000.0', roi: '0.238',
  matched_qty: '80.0', unmatched_qty: '0.0',
  matches: [
    { buy_transaction_id: 1, buy_date: '2024-01-01T00:00:00Z', buy_price: '100.0',
      unit_cost: '100.0', matched_qty: '60.0', lot_qty_before_match: '60.0',
      cost_basis: '6000.0', proceeds: '7800.0', pnl: '1800.0',
      holding_days: 335, term: null },
  ],
}

describe('normalizeSellPnl', () => {
  it('converts decimal strings to numbers', () => {
    const d = normalizeSellPnl(raw)
    expect(d.sold_qty).toBe(80)
    expect(d.realized_pnl).toBe(2000)
    expect(d.matches[0].matched_qty).toBe(60)
    expect(d.matches[0].pnl).toBe(1800)
    expect(d.matches[0].holding_days).toBe(335)
  })

  it('keeps roi null when there is no cost basis', () => {
    const d = normalizeSellPnl({ ...raw, roi: null, cost_basis: '0.0' })
    expect(d.roi).toBeNull()
  })

  it('reads a present roi as a number', () => {
    expect(normalizeSellPnl(raw).roi).toBeCloseTo(0.238)
  })

  it('survives a malformed payload without throwing', () => {
    const d = normalizeSellPnl(null)
    expect(d.matches).toEqual([])
    expect(d.realized_pnl).toBe(0)
    expect(d.roi).toBeNull()
  })

  it('passes term through', () => {
    const d = normalizeSellPnl({
      ...raw,
      matches: [{ ...raw.matches[0], term: 'SHORT' }],
    })
    expect(d.matches[0].term).toBe('SHORT')
  })

  it('coerces numeric ids that arrive as strings', () => {
    // `as number` is a compile-time cast: a payload with "transaction_id":"42"
    // would otherwise hand the UI a string typed as number, and `?? 0` does not
    // catch that because a string is neither null nor undefined.
    const d = normalizeSellPnl({
      ...raw,
      transaction_id: '42', asset_id: '7', threshold_days: '30',
      matches: [{ ...raw.matches[0], holding_days: '335', buy_transaction_id: '11' }],
    })
    expect(d.transaction_id).toBe(42)
    expect(d.asset_id).toBe(7)
    expect(d.threshold_days).toBe(30)
    expect(d.matches[0].holding_days).toBe(335)
    // The nested id is a React key and the only handle back to the buy row, so it
    // needs the same runtime coercion as the top-level ones.
    expect(d.matches[0].buy_transaction_id).toBe(11)
  })
})

describe('normalizeTaxSummary run-context fields', () => {
  it('keeps strategy and threshold_days when the payload echoes them', () => {
    const s = normalizeTaxSummary({
      currency: 'EUR', strategy: 'HYBRID', threshold_days: 45,
      total_portfolio_value: '10.00', total_realized_pnl: '1234.565',
      total_unrealized_pnl: '0.00', assets: [],
    });
    expect(s.strategy).toBe('HYBRID');
    expect(s.threshold_days).toBe(45);
  });

  it('leaves both undefined when the payload omits them (the empty-portfolio branch)', () => {
    const s = normalizeTaxSummary({
      total_portfolio_value: 0, total_realized_pnl: 0, total_unrealized_pnl: 0, assets: [],
    });
    expect(s.strategy).toBeUndefined();
    expect(s.threshold_days).toBeUndefined();
  });
})
