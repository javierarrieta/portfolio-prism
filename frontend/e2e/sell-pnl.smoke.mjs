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

import { readFileSync } from 'node:fs'

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
  // Wait for the payload, not for a heading: while the request is in flight the dialog
  // renders the bare "Sell P&L" title, so /Sell P&L/ matches immediately and readModal()
  // then reads a dialog with no figures in it. The strategy note only exists once data has
  // landed, so waiting on it is waiting on the fetch.
  await page.getByTestId('strategy-note').waitFor({ timeout: 10000 })

  const readModal = () => page.evaluate(() => {
    const modal = document.querySelector('[role="dialog"]')
    if (!modal) throw new Error('no [role=dialog] on the page')
    const headers = [...modal.querySelectorAll('thead th')].map((th) => th.textContent.trim())
    // Index by header text: a hardcoded r[5] silently reads the wrong column the
    // moment anyone inserts one.
    const pnlCol = headers.findIndex((h) => /P&L/i.test(h))
    const dateCol = headers.findIndex((h) => /Buy Date/i.test(h))
    return {
      pnlCol, dateCol,
      headers,
      realizedShown: [...modal.querySelectorAll('div')].find(
        (d) => /^\$[\d,]+\.\d{2}$/.test(d.textContent.trim()) && getComputedStyle(d).fontWeight === '700',
      )?.textContent.trim(),
      roi: modal.querySelector('[data-testid="roi"]')?.textContent.trim(),
      strategyNote: modal.querySelector('[data-testid="strategy-note"]')?.textContent.trim(),
      rows: [...modal.querySelectorAll('tbody tr')].map((r) => [...r.querySelectorAll('td')].map((td) => td.textContent.trim())),
      // Wrapping is invisible to textContent -- "2024-01-01" reads the same whether
      // it sits on one line or three. A Range's client rects are per line-box, so
      // more than one means the text is broken across lines. Only a browser can
      // see this; jsdom has no layout engine.
      dateRects: [...modal.querySelectorAll('tbody tr')].map((tr) => {
        const td = tr.querySelectorAll('td')[dateCol]
        const r = document.createRange()
        r.selectNodeContents(td)
        return r.getClientRects().length
      }),
    }
  })
  let dom = await readModal()
  console.log('dom  realized=' + dom.realizedShown + ' roi=' + dom.roi + ' note=' + JSON.stringify(dom.strategyNote))
  console.log('dom  rows=' + JSON.stringify(dom.rows))

  if (process.env.SMOKE_SHOT) await page.screenshot({ path: process.env.SMOKE_SHOT, fullPage: true })

  // 1. What the modal shows is what the Holdings endpoint reports.
  if (dom.realizedShown == null) fail('could not locate the realized figure in the modal')
  else if (money(dom.realizedShown) !== truth.realized) fail(`modal shows ${dom.realizedShown}, tax-summary reports ${truth.realized}`)
  else ok(`modal realized ${dom.realizedShown} matches tax-summary`)

  // 2. The per-lot column adds up to it. This is the invariant the feature exists for.
  if (dom.pnlCol < 0) fail(`no P&L column among ${JSON.stringify(dom.headers)}`)
  const column = dom.rows.map((r) => money(r[dom.pnlCol]))
  const sum = column.reduce((a, b) => a + b, 0)
  if (dom.rows.length !== truth.lots.length) fail(`modal shows ${dom.rows.length} lot rows, endpoint returned ${truth.lots.length}`)
  if (sum !== truth.realized) fail(`per-lot column ${column.join(' + ')} = ${sum}, not ${truth.realized}`)
  else ok(`per-lot column ${column.join(' + ')} = ${sum}`)

  // 3. Attribution is named, and named as FIFO because that is what ran.
  if (!/realized · FIFO/.test(dom.strategyNote ?? '')) fail(`strategy note unexpected: ${dom.strategyNote}`)
  else ok('strategy note names the method that ran')

  // 4. Dates survive their column at every width the layout claims to support.
  //    Asserting at one viewport only proves the width the test happened to pick.
  for (const width of [1400, 1100, 900]) {
    await page.setViewportSize({ width, height: 1000 })
    const wrapped = (await readModal()).dateRects.filter((n) => n > 1).length
    if (wrapped) fail(`at ${width}px, ${wrapped} buy date(s) wrap -- column too narrow`)
    else ok(`buy dates on one line at ${width}px`)
  }
  await page.setViewportSize({ width: 1400, height: 1000 })
  dom = await readModal()

  // 5. Escape closes the modal, and a reopened one reflects the current strategy.
  //    The backdrop covers the strategy select, so changing strategy with the modal
  //    open is not something a user can do -- this drives the reachable path:
  //    close, switch, reopen. A stale memo would show FIFO figures under a LIFO
  //    label. LIFO on this fixture is 1600 against FIFO's 2000, so it cannot be missed.
  await page.keyboard.press('Escape')
  await page.locator('[role="dialog"]').waitFor({ state: 'detached', timeout: 5000 })
  ok('Escape closed the dialog')

  // The strategy select lives on the Holdings & Tax lots tab, so switching means
  // leaving the ledger tab and coming back.
  await page.getByText('Holdings & Tax lots').click()
  // Locate by the accessible name. filter({ hasText: 'LIFO' }) cannot work: a
  // select's innerText is only its *selected* option, so it never contains LIFO
  // until after the click that is trying to find it.
  await page.getByLabel('Tax accounting method').selectOption('LIFO')
  await page.getByText('Transaction Ledger').click()
  await page.getByRole('button', { name: /realized P&L for AAPL/i }).click()
  // Same wait as the first open, and the same reason: this one was measured failing with
  // "after reopening, note is undefined" because the heading matched while the LIFO request
  // was still in flight. The download step below reads this same `lifo` payload, so it
  // inherits the race.
  await page.getByTestId('strategy-note').waitFor({ timeout: 10000 })
  const lifo = await readModal()
  if (!/realized \u00b7 LIFO/.test(lifo.strategyNote ?? '')) fail(`after reopening, note is ${lifo.strategyNote}`)
  else if (money(lifo.realizedShown) !== 1600) fail(`after switching to LIFO the modal shows ${lifo.realizedShown}, expected $1,600.00`)
  else ok(`reopened under LIFO: ${lifo.realizedShown}`)

  // 6. The file the browser actually downloads. The vitest component test cannot reach this
  //    far: jsdom implements neither URL.createObjectURL nor Blob.text(), so it reads the
  //    Blob back through FileReader and stops at the bytes. Here the download is real, so
  //    the anchor, the object URL and the suggested filename are all genuinely exercised.
  //    The modal is showing LIFO at this point, which is the point: the export must reflect
  //    the payload on screen, not the FIFO one the modal fetched first.
  //    Imported here rather than at the top, for the same reason playwright-core is: a module
  //    that fails to resolve should not bury the "CHROMIUM is not set" message below it.
  const Papa = (await import('papaparse')).default
  const expectedName = 'AAPL_2024-12-01_sell-pnl.csv'
  const downloadPromise = page.waitForEvent('download')
  await page.getByRole('button', { name: /download csv/i }).click()
  const download = await downloadPromise
  const gotName = download.suggestedFilename()
  if (gotName !== expectedName) fail(`download offered as ${gotName}, expected ${expectedName}`)
  else ok(`download offered as ${gotName}`)

  const filePath = await download.path()
  if (!filePath) fail('download produced no file (acceptDownloads off?)')
  else {
    const csvText = readFileSync(filePath, 'utf8')
    const parsed = Papa.parse(csvText, { header: true, skipEmptyLines: true })
    if (parsed.errors.length) fail(`csv did not parse cleanly: ${parsed.errors.map((e) => e.message).join(' | ')}`)
    const rows = parsed.data
    const sum = rows.reduce((a, r) => (r.lot_pnl === '' ? a : a + parseFloat(r.lot_pnl)), 0)
    const realized = parseFloat(rows[0]?.realized_pnl)
    // The screen rounds to 2dp (formatCurrency) and the file does not, so this compares to
    // the display's own resolution. Exact equality here only held because the fixture is
    // round; one fractional cent in a fixture would have failed on formatting, not a bug.
    const shown = money(lifo.realizedShown)
    if (rows.length !== 2) fail(`csv has ${rows.length} lot rows, expected 2`)
    else if (rows.some((r) => r.strategy !== 'LIFO')) fail('csv rows do not all say LIFO -- exported a stale payload')
    else if (csvText.includes('$')) fail('csv contains a currency symbol: built from a display formatter')
    else if (Math.abs(realized - shown) > 0.005) fail(`csv realized_pnl ${realized} does not match the ${shown} on screen`)
    else if (Math.abs(sum - realized) > 1e-9) fail(`csv lot pnl sums to ${sum}, not ${realized}`)
    else ok(`csv matches the screen: 2 LIFO lots summing to ${realized}`)
  }

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
