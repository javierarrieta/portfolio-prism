// Headless browser smoke for the sell P&L breakdown.
//
// This is the only test in the repo that exercises the real seam: a real browser,
// the real Rocket API, and a real SQLite file. The vitest suite mocks this
// endpoint with MSW; the cargo tests never render anything. Neither can see a
// broken fetch URL, a proxy that is not configured, or a column that is too
// narrow to hold a date.
//
// It seeds its own fixture through the REST API, so nothing here depends on
// demo data. Point DATABASE_URL at a throwaway file (run.sh does) and it never
// touches your real portfolio.db.
//
// Required env:
//   CHROMIUM   path to a chromium-compatible browser binary
// Optional env:
//   BASE_URL   default http://localhost:5173
//   API_URL    default http://127.0.0.1:8000/api
//   SMOKE_SHOT set to a path to write a screenshot of the open modal

const BASE_URL = process.env.BASE_URL ?? 'http://localhost:5173'
const API_URL = process.env.API_URL ?? 'http://127.0.0.1:8000/api'
const CHROMIUM = process.env.CHROMIUM

const failures = []
const fail = (m) => { failures.push(m); console.error('FAIL: ' + m) }
const ok = (m) => console.log('ok   ' + m)
const money = (s) => parseFloat(String(s).replace(/[^0-9.\-]/g, ''))

async function api(path, init) {
  const res = await fetch(API_URL + path, init)
  if (!res.ok) throw new Error(`${init?.method ?? 'GET'} ${path} -> ${res.status}`)
  return res.json()
}

async function seed() {
  const stamp = Date.now()
  const portfolio = await api('/portfolios', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ name: `E2E Smoke ${stamp}`, description: 'headless smoke', currency: 'USD' }),
  })
  const asset = await api(`/portfolios/${portfolio.id}/assets`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ symbol: 'AAPL', name: 'Apple Inc.', asset_type: 'STOCK', sector: 'Technology', currency: 'USD', isin: '' }),
  })
  // Two buys and one sell that straddles both lots, so FIFO must split it 60/20.
  const txs = [
    ['BUY', '60', '100', '0', '2024-01-01T00:00:00Z'],
    ['BUY', '40', '120', '0', '2024-06-01T00:00:00Z'],
    ['SELL', '80', '130', '0', '2024-12-01T00:00:00Z'],
  ]
  for (const [type, quantity, price, fee, date] of txs) {
    await api(`/portfolios/${portfolio.id}/assets/${asset.id}/transactions`, {
      method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ type, quantity, price, fee, date }),
    })
  }
  return { portfolioId: portfolio.id, assetId: asset.id }
}

const browser = await (await import('playwright-core')).chromium.launch({
  executablePath: CHROMIUM,
  args: ['--no-sandbox', '--disable-dev-shm-usage'],
})
const page = await browser.newPage({ viewport: { width: 1400, height: 1000 } })
const jsErrors = []
page.on('pageerror', (e) => jsErrors.push('pageerror: ' + e.message))
page.on('console', (m) => { if (m.type() === 'error') jsErrors.push('console.error: ' + m.text()) })

try {
  if (!CHROMIUM) throw new Error('CHROMIUM is not set -- point it at a chromium binary')
  const { portfolioId } = await seed()

  await page.goto(BASE_URL, { waitUntil: 'networkidle' })

  // A browser with no fonts renders zero-width text: the page looks structurally
  // fine, every glyph is missing, and a visibility wait fails in a way that reads
  // like a broken component. Measure it here so the error names the real cause.
  const glyphWidth = await page.evaluate(() => {
    const s = document.createElement('span')
    s.textContent = 'Revenue 1234'
    s.style.cssText = 'position:absolute;visibility:hidden;white-space:pre;font-size:32px'
    document.body.appendChild(s)
    const w = s.getBoundingClientRect().width
    s.remove()
    return w
  })
  if (glyphWidth === 0) {
    throw new Error(
      'chromium rendered zero-width text: fontconfig found no fonts. ' +
      'On NixOS, run via e2e/run.sh, which sets FONTCONFIG_FILE for you.',
    )
  }

  // Ground truth, read over the same API the Holdings tab uses.
  const summary = await api(`/portfolios/${portfolioId}/tax-summary?strategy=FIFO&threshold_days=30`)
  const realized = parseFloat(summary.assets.find((a) => a.symbol === 'AAPL').realized_pnl)
  const txList = await api(`/portfolios/${portfolioId}/transactions`)
  const sellId = txList.find((t) => t.type === 'SELL').id
  const pnl = await api(`/transactions/${sellId}/pnl?strategy=FIFO&threshold_days=30`)
  const truth = { realized, endpoint: parseFloat(pnl.realized_pnl), lots: pnl.matches.map((m) => parseFloat(m.pnl)) }
  console.log('api  ' + JSON.stringify(truth))

  await page.getByText('Transaction Ledger').click()
  await page.getByRole('button', { name: /realized P&L for AAPL/i }).click()
  await page.getByText(/Sell P&L/).waitFor({ timeout: 10000 })

  const dom = await page.evaluate(() => {
    const modal = [...document.querySelectorAll('.glass-card')].find((el) => el.textContent.includes('Sell P&L'))
    if (!modal) throw new Error('modal .glass-card not found')
    return {
      rows: [...modal.querySelectorAll('tbody tr')].map((r) => [...r.querySelectorAll('td')].map((td) => td.textContent.trim())),
      realizedShown: [...modal.querySelectorAll('div')].find(
        (d) => /^\$[\d,]+\.\d{2}$/.test(d.textContent.trim()) && getComputedStyle(d).fontWeight === '700',
      )?.textContent.trim(),
      roi: modal.querySelector('[data-testid="roi"]')?.textContent.trim(),
      strategyNote: modal.querySelector('[data-testid="strategy-note"]')?.textContent.trim(),
      // Wrapping is invisible to textContent -- "2024-01-01" reads the same whether
      // it sits on one line or three. A Range's client rects are per line-box, so
      // more than one means the text is broken across lines. Only a browser can
      // see this; jsdom has no layout engine.
      dateRects: [...modal.querySelectorAll('tbody tr td:first-child')].map((td) => {
        const r = document.createRange()
        r.selectNodeContents(td)
        return r.getClientRects().length
      }),
    }
  })
  console.log('dom  realized=' + dom.realizedShown + ' roi=' + dom.roi + ' note=' + JSON.stringify(dom.strategyNote))
  console.log('dom  rows=' + JSON.stringify(dom.rows))

  if (process.env.SMOKE_SHOT) await page.screenshot({ path: process.env.SMOKE_SHOT, fullPage: true })

  // 1. What the modal shows is what the Holdings endpoint reports.
  if (dom.realizedShown == null) fail('could not locate the realized figure in the modal')
  else if (money(dom.realizedShown) !== truth.realized) fail(`modal shows ${dom.realizedShown}, tax-summary reports ${truth.realized}`)
  else ok(`modal realized ${dom.realizedShown} matches tax-summary`)

  // 2. The per-lot column adds up to it. This is the invariant the feature exists for.
  const column = dom.rows.map((r) => money(r[5]))
  const sum = column.reduce((a, b) => a + b, 0)
  if (dom.rows.length !== truth.lots.length) fail(`modal shows ${dom.rows.length} lot rows, endpoint returned ${truth.lots.length}`)
  if (sum !== truth.realized) fail(`per-lot column ${column.join(' + ')} = ${sum}, not ${truth.realized}`)
  else ok(`per-lot column ${column.join(' + ')} = ${sum}`)

  // 3. Attribution is named, and named as FIFO because that is what ran.
  if (!/realized · FIFO/.test(dom.strategyNote ?? '')) fail(`strategy note unexpected: ${dom.strategyNote}`)
  else ok('strategy note names the method that ran')

  // 4. Dates survive their column. jsdom has no layout, so only a browser sees this.
  if (dom.dateRects.some((n) => n > 1)) fail(`buy date wraps across ${Math.max(...dom.dateRects)} line boxes -- the column is too narrow`)
  else ok('buy dates render on one line')

  if (jsErrors.length) fail(jsErrors.join(' | '))
  else ok('no page errors')
} catch (e) {
  fail(e.message)
  if (process.env.SMOKE_SHOT) await page.screenshot({ path: process.env.SMOKE_SHOT, fullPage: true }).catch(() => {})
} finally {
  await browser.close()
}

if (failures.length) {
  console.error(`\n${failures.length} failure(s)`)
  process.exit(1)
}
console.log('\nPASS: browser-rendered breakdown reconciles with the API it called.')
