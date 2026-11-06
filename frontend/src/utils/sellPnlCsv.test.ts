import { describe, it, expect } from 'vitest'
import Papa from 'papaparse'
import { sellPnlToCsv, sellPnlFilename } from './sellPnlCsv'
import type { LotMatch, SellPnlBreakdown } from '../types'

// Numbers, not the decimal strings the API sends: sellPnlToCsv takes the output of
// normalizeSellPnl, which has already coerced every money field through Number().
const lot = (over: Partial<LotMatch> = {}): LotMatch => ({
  buy_transaction_id: 1,
  buy_date: '2024-01-01T00:00:00Z',
  buy_price: 100,
  unit_cost: 100,
  matched_qty: 50,
  lot_qty_before_match: 50,
  cost_basis: 5000,
  proceeds: 6500,
  pnl: 1500,
  holding_days: 335,
  term: null,
  ...over,
})

const breakdown = (over: Partial<SellPnlBreakdown> = {}): SellPnlBreakdown => ({
  transaction_id: 42,
  asset_id: 1,
  symbol: 'AAPL',
  asset_type: 'STOCK',
  strategy: 'FIFO',
  threshold_days: 30,
  currency: 'USD',
  asset_currency: 'USD',
  fx_applied: false,
  split_applied: false,
  sell_date: '2024-12-01T00:00:00Z',
  sold_qty: 80,
  price: 130,
  fee: 0,
  proceeds: 10400,
  cost_basis: 8400,
  realized_pnl: 2000,
  roi: 0.238,
  matched_qty: 80,
  unmatched_qty: 0,
  matches: [
    lot(),
    lot({
      buy_transaction_id: 2,
      buy_date: '2024-06-01T00:00:00Z',
      matched_qty: 30,
      lot_qty_before_match: 30,
      cost_basis: 3400,
      proceeds: 3900,
      pnl: 500,
      holding_days: 183,
    }),
  ],
  ...over,
})

/** Parse back into objects the way a spreadsheet would, so tests compare values, not text. */
const dataRows = (csv: string) =>
  Papa.parse<Record<string, string>>(csv, { header: true, skipEmptyLines: true }).data

const headerRow = (csv: string) => Papa.parse<string[]>(csv, { skipEmptyLines: true }).data[0]

const sumCol = (rows: Record<string, string>[], col: string) =>
  rows.reduce((acc, r) => (r[col] === '' ? acc : acc + Number(r[col])), 0)

describe('sellPnlToCsv', () => {
  it('writes one row per lot with the sell context repeated on each', () => {
    const rows = dataRows(sellPnlToCsv(breakdown()))
    expect(rows).toHaveLength(2)
    for (const r of rows) {
      expect(r.transaction_id).toBe('42')
      expect(r.symbol).toBe('AAPL')
      expect(r.sell_date).toBe('2024-12-01T00:00:00Z')
    }
    expect(rows.map((r) => r.buy_date)).toEqual(['2024-01-01T00:00:00Z', '2024-06-01T00:00:00Z'])
  })

  it('writes money cells as raw numbers with no separator and no currency symbol', () => {
    // formatCurrency would hand back "$1,234,567.891": the commas shift columns and the
    // symbol makes the cell text. The file must hold the number itself.
    const csv = sellPnlToCsv(breakdown({
      proceeds: 1234567.891,
      matches: [lot({ proceeds: 9876543.21, cost_basis: 12345.6 })],
    }))
    expect(csv).toContain('1234567.891')
    expect(csv).toContain('9876543.21')
    expect(csv).not.toMatch(/\$/)
    // Assert on cells, not the raw line: adjacent numbers like `891,8400` are two columns
    // but look exactly like a thousands separator to a regex run over the whole file.
    const moneyCols = ['price_in_asset_currency', 'fee_in_asset_currency', 'proceeds', 'cost_basis', 'realized_pnl',
      'buy_price', 'lot_cost_basis', 'lot_proceeds', 'lot_pnl']
    for (const row of dataRows(csv)) {
      for (const col of moneyCols) expect(row[col]).toMatch(/^-?\d+(\.\d+)?$/)
    }
    expect(Number(dataRows(csv)[0].lot_proceeds)).toBe(9876543.21)
  })

  it('names the as-entered columns with the currency they are in', () => {
    // price and fee arrive as entered, so they are in asset_currency, while every other
    // money column on the file is in the base currency. A bare `price` header is the one
    // thing a tax tool could silently misread on a foreign-currency sell.
    const csv = sellPnlToCsv(breakdown({ currency: 'EUR', asset_currency: 'USD', fx_applied: true }))
    const h = headerRow(csv)
    expect(h).toContain('price_in_asset_currency')
    expect(h).toContain('fee_in_asset_currency')
    expect(h).not.toContain('price')
    expect(h).not.toContain('fee')

    const row = dataRows(csv)[0]
    expect([row.price_in_asset_currency, row.fee_in_asset_currency]).toEqual(['130', '0'])
    // The two currency columns say which string each of those names refers to.
    expect([row.currency, row.asset_currency]).toEqual(['EUR', 'USD'])
  })

  it('quotes a symbol containing a comma', () => {
    const csv = sellPnlToCsv(breakdown({ symbol: 'ACME,INC' }))
    expect(csv).toContain('"ACME,INC"')
    expect(dataRows(csv)[0].symbol).toBe('ACME,INC')
  })

  it('writes the price the screen shows alongside the fee-loaded unit cost', () => {
    // The modal's Buy price column is buy_price; unit_cost additionally carries the
    // amortised buy fee (backend_rust/src/schemas.rs), so with any fee they differ and the
    // file is only reconcilable against the screen if both are there.
    const csv = sellPnlToCsv(breakdown({ matches: [lot({ buy_price: 100, unit_cost: 100.5 })] }))
    expect([dataRows(csv)[0].buy_price, dataRows(csv)[0].unit_cost]).toEqual(['100', '100.5'])
  })

  it('neutralises a symbol that a spreadsheet would execute', () => {
    // A symbol is free text the whole way down: csvValidator only checks it is non-empty and
    // the backend uppercases it without filtering, so somebody else's imported CSV can put
    // a formula in one. Excel and Sheets run a cell starting with = + - @ or a tab, and
    // Papa quotes only on comma/quote/newline, so nothing else stands between the two. The
    // leading apostrophe is what makes the cell text. Numbers keep their sign -- a guard
    // that prefixed -1500 would break the numeric column the file exists to be.
    for (const lead of ['=', '+', '-', '@', '\t']) {
      expect(dataRows(sellPnlToCsv(breakdown({ symbol: `${lead}EVIL` })))[0].symbol).toBe(`'${lead}EVIL`)
    }
    expect(dataRows(sellPnlToCsv(breakdown({ matches: [lot({ pnl: -1500 })] })))[0].lot_pnl).toBe('-1500')
  })

  it('writes a tiny number in fixed notation, not exponent notation', () => {
    // String(1e-7) is '1e-7', and a small pnl on a large basis reaches it without trying.
    const csv = sellPnlToCsv(breakdown({ roi: 1e-7, matches: [lot({ pnl: 0.00000001 })] }))
    expect(csv).not.toMatch(/e[-+]/)
    expect([dataRows(csv)[0].roi, dataRows(csv)[0].lot_pnl]).toEqual(['0.0000001', '0.00000001'])
  })

  it('leaves a null roi as a blank cell', () => {
    const rows = dataRows(sellPnlToCsv(breakdown({ cost_basis: 0, roi: null })))
    expect(rows[0].roi).toBe('')
  })

  it('leaves a null term blank and keeps a real one', () => {
    const rows = dataRows(sellPnlToCsv(breakdown({
      strategy: 'HYBRID',
      matches: [lot({ term: 'LONG' }), lot({ term: 'SHORT' })],
    })))
    expect(rows.map((r) => r.term)).toEqual(['LONG', 'SHORT'])
    expect(dataRows(sellPnlToCsv(breakdown()))[0].term).toBe('')
  })

  it('writes caveats as columns, with no leading comment row', () => {
    // A `#` line breaks naive CSV readers, so strategy/fx/split ride along on every row.
    const csv = sellPnlToCsv(breakdown({ strategy: 'HYBRID', threshold_days: 365, fx_applied: true, split_applied: true }))
    expect(csv.startsWith('#')).toBe(false)
    const h = headerRow(csv)
    for (const col of ['strategy', 'threshold_days', 'fx_applied', 'split_applied', 'currency']) {
      expect(h).toContain(col)
    }
    const row = dataRows(csv)[0]
    expect([row.strategy, row.threshold_days, row.fx_applied, row.split_applied]).toEqual(['HYBRID', '365', 'true', 'true'])
  })

  it('writes a header-only file when nothing matched', () => {
    const csv = sellPnlToCsv(breakdown({ matches: [], matched_qty: 0, unmatched_qty: 0, realized_pnl: 0, roi: null }))
    expect(headerRow(csv)[0]).toBe('row_type')
    expect(dataRows(csv)).toHaveLength(0)
  })

  it('sums the pnl column to realized_pnl', () => {
    // 0.1 + 0.2 is 0.30000000000000004 in a float, so the file's own total is compared with
    // a tolerance. Exact equality here would be a lie about binary floating point, and the
    // backend's full-precision decimals were already coerced to numbers by normalizeSellPnl.
    const csv = sellPnlToCsv(breakdown({
      realized_pnl: 0.3,
      matches: [lot({ pnl: 0.1 }), lot({ pnl: 0.2 })],
    }))
    const rows = dataRows(csv)
    const realized = Number(rows[0].realized_pnl)
    expect(sumCol(rows, 'lot_pnl')).toBeCloseTo(realized, 9)
  })

  it('gives unmatched quantity its own row without polluting the pnl sum', () => {
    const csv = sellPnlToCsv(breakdown({ sold_qty: 100, matched_qty: 80, unmatched_qty: 20 }))
    const rows = dataRows(csv)
    expect(rows.map((r) => r.row_type)).toEqual(['LOT', 'LOT', 'UNMATCHED'])

    const unmatched = rows[rows.length - 1]
    expect(unmatched.lot_matched_qty).toBe('20')
    // Blank, not 0: a spreadsheet SUM skips blanks, so the pnl column still totals exactly.
    expect([unmatched.lot_pnl, unmatched.lot_cost_basis, unmatched.lot_proceeds]).toEqual(['', '', ''])
    expect(unmatched.buy_date).toBe('')
    expect(unmatched.unmatched_qty).toBe('20')

    expect(sumCol(rows, 'lot_pnl')).toBeCloseTo(2000, 9)
    // The other invariant that holds over the whole column: every sold unit is accounted for.
    expect(sumCol(rows, 'lot_matched_qty')).toBe(100)
  })

  it('keeps CRLF line endings', () => {
    // Papa.unparse's default, and what RFC 4180 asks for. Pinned so nobody "fixes" it.
    expect(sellPnlToCsv(breakdown())).toContain('\r\n')
  })
})

describe('sellPnlFilename', () => {
  it('names the file from the symbol and the sell date', () => {
    expect(sellPnlFilename(breakdown())).toBe('AAPL_2024-12-01_sell-pnl.csv')
  })

  it('takes the date from the raw string, not a local-time conversion', () => {
    // new Date('2024-12-01T00:30:00Z') is 2024-11-30 west of UTC and
    // '2024-12-01T23:30:00Z' is 2024-12-02 east of it. Either conversion silently
    // mislabels the file; the first 10 characters of the payload string cannot.
    // Rewriting TZ bites only because Node re-reads it since v16; on an older runtime this
    // loop would pass vacuously, so it proves the slice rather than the zone handling.
    const realTZ = process.env.TZ
    try {
      for (const zone of ['America/Los_Angeles', 'Asia/Tokyo', 'UTC']) {
        process.env.TZ = zone
        for (const iso of ['2024-12-01T00:30:00Z', '2024-12-01T23:30:00Z']) {
          expect(sellPnlFilename(breakdown({ sell_date: iso }))).toBe('AAPL_2024-12-01_sell-pnl.csv')
        }
      }
    } finally {
      if (realTZ === undefined) delete process.env.TZ
      else process.env.TZ = realTZ
    }
  })

  it('replaces characters that are not filename-safe', () => {
    // A symbol is user-entered; 'BRK.B' must survive while a path separator must not.
    expect(sellPnlFilename(breakdown({ symbol: 'BRK.B' }))).toBe('BRK.B_2024-12-01_sell-pnl.csv')
    expect(sellPnlFilename(breakdown({ symbol: 'a/b:c' }))).toBe('a_b_c_2024-12-01_sell-pnl.csv')
  })

  it('falls back rather than emitting an empty filename part', () => {
    expect(sellPnlFilename(breakdown({ sell_date: '' }))).toBe('AAPL_unknown_sell-pnl.csv')
    expect(sellPnlFilename(breakdown({ symbol: '' }))).toBe('SELL_2024-12-01_sell-pnl.csv')
  })
})
