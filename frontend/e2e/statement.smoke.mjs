// Headless browser smoke for the printable statement. Same harness as
// sell-pnl.smoke.mjs: a real browser, the real Rocket API, a real SQLite file.
// This is the only layer that sees real print CSS -- jsdom does not apply
// @media print, and there is no browser job in CI (issue #45).
//
// It asserts NO figure. Figures come from a live price provider; when it is
// unreachable the fallback returns zero, and a smoke that asserted figures
// would print zeros and still pass. It pins structure, visibility and colour.
//
// Required env: CHROMIUM   path to a chromium-compatible browser binary
// Optional env: BASE_URL   default http://localhost:5173
//               API_URL    default http://127.0.0.1:8000/api
//               SMOKE_SHOT set to a path to write a full-page screenshot when the run fails

const BASE_URL = process.env.BASE_URL ?? 'http://localhost:5173'
const API_URL = process.env.API_URL ?? 'http://127.0.0.1:8000/api'
const CHROMIUM = process.env.CHROMIUM

const failures = []
const fail = (m) => { failures.push(m); console.error('FAIL: ' + m) }
const ok = (m) => console.log('ok   ' + m)

async function api(path, init) {
  const res = await fetch(API_URL + path, init)
  if (!res.ok) throw new Error(`${init?.method ?? 'GET'} ${path} -> ${res.status}`)
  return res.json()
}

async function seed() {
  const stamp = Date.now()
  const portfolio = await api('/portfolios', {
    method: 'POST', headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ name: `Statement Smoke ${stamp}`, description: 'headless smoke', currency: 'USD' }),
  })
  // Two USD holdings in a USD portfolio. Same-currency is still a hard requirement, but
  // not for the reason first written here: since the pricing layer landed a
  // foreign-currency asset no longer 500s tax-summary outright (fx_symbol now yields
  // EURUSD=X, and get_rate short-circuits a same-currency pair). Rate lookups go to the
  // live provider, so a fixture that needs FX can fail outright on a rate nobody can
  // answer, or be priced at whatever older close the lookback substituted -- and this
  // smoke asserts no figure, so it must not depend on one.
  //
  // Width is the other half of the fixture. The first version seeded two STOCK assets at
  // $100-130, which is the narrowest table this endpoint can produce -- widest unbreakable
  // token "STOCK", ~40px -- so the overflow check at the bottom of this file passed for a
  // reason unrelated to the risk it exists to catch. MUTUAL_FUND is the widest asset_type
  // the app can store (types.ts:9) and the widest unbreakable token the table prints, and
  // the recorded realized loss puts a leading-minus six-figure amount in the Realized
  // column: 1200 x (5 - 120) = -$138,000.00. Both are deterministic -- realized P&L is
  // replayed from these transactions, never read from the live provider -- so this still
  // adds no figure assertion. At the first print padding this fixture measured ~632px
  // against the 608px box and failed by ~24px; at 4px 6px it should land near 596px. If it
  // fails on a real run that IS the finding: do not widen the viewport and do not narrow
  // the fixture.
  const holdings = [
    { symbol: 'AAPL', name: 'Apple Inc.', asset_type: 'STOCK', txs: [
      ['BUY', '60', '100', '2024-01-01T00:00:00Z'],
      ['SELL', '20', '130', '2024-12-01T00:00:00Z'],
    ] },
    { symbol: 'MSFT', name: 'Microsoft', asset_type: 'MUTUAL_FUND', txs: [
      ['BUY', '1500', '120', '2024-01-01T00:00:00Z'],
      ['SELL', '1200', '5', '2024-12-01T00:00:00Z'],
    ] },
  ]
  for (const { symbol, name, asset_type, txs } of holdings) {
    const asset = await api(`/portfolios/${portfolio.id}/assets`, {
      method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ symbol, name, asset_type, sector: 'Technology', currency: 'USD', isin: '' }),
    })
    for (const [type, quantity, price, date] of txs) {
      await api(`/portfolios/${portfolio.id}/assets/${asset.id}/transactions`, {
        method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ type, quantity, price, fee: '0', date }),
      })
    }
  }
  return portfolio
}

const portfolio = await seed()
// Named in the log because the app auto-selects the first portfolio it is given, and
// a run that printed someone else's portfolio would look identical to one that did not.
console.log('seed ' + JSON.stringify({ id: portfolio.id, name: portfolio.name, currency: portfolio.currency }))
const { chromium } = await import('playwright-core')
const browser = await chromium.launch({
  executablePath: CHROMIUM, args: ['--no-sandbox', '--disable-dev-shm-usage'],
})
const page = await browser.newPage({ viewport: { width: 1400, height: 1000 } })

try {
  await page.goto(BASE_URL, { waitUntil: 'networkidle' })
  // The nav items are <li onClick> with no role, so getByText is the only handle.
  await page.getByText('Printable Statement').click()
  const statement = page.locator('[data-print="statement"]')
  await statement.waitFor({ state: 'visible' })
  ok('statement rendered')

  // BEFORE emulateMedia, on screen: the sidebar must be visible. print.css keeps its
  // hide rules inside `@media print`, but PrintCss.test.ts reads the file as text and
  // cannot see @media membership -- a hide rule duplicated outside the block passes every
  // one of its tests while blanking the sidebar on screen. This is the assertion that
  // catches that, and it is only meaningful while the media type is still screen.
  const onScreen = await page.locator('.sidebar').boundingBox()
  const onScreenDisplay = await page.locator('.sidebar').evaluate((el) => getComputedStyle(el).display)
  if (!onScreen || onScreen.width === 0 || onScreen.height === 0 || onScreenDisplay === 'none') {
    fail(`sidebar not visible on screen (box=${JSON.stringify(onScreen)} display=${onScreenDisplay}): a hide rule escaped @media print`)
  } else {
    ok('sidebar visible on screen, so the hide rules are inside @media print')
  }

  // Same argument, for the two rules that would be visible in the RUNNING APP if they
  // escaped @media print: a white sidebar and a shrunken table. PrintCss.test.ts pins both
  // as text and cannot see @media membership at all, so either escaping is green everywhere
  // except here.
  const sidebarBg = await page.locator('.sidebar').evaluate((el) => getComputedStyle(el).backgroundColor)
  if (sidebarBg === 'rgb(255, 255, 255)') {
    fail(`sidebar computes backgroundColor ${sidebarBg} on screen: the print background rule escaped @media print`)
  } else {
    ok(`sidebar keeps its dark background on screen (${sidebarBg})`)
  }

  const tdPadding = await page.locator('[data-print="statement"] td').first()
    .evaluate((el) => getComputedStyle(el).padding)
  if (tdPadding !== '16px') {
    fail(`statement td computes padding ${tdPadding} on screen, expected 16px: the print typography rule escaped @media print`)
  } else {
    ok('statement table keeps its 16px on-screen padding')
  }

  await page.emulateMedia({ media: 'print' })

  const box = await statement.boundingBox()
  if (!box || box.height === 0) fail('statement has no box under print media')

  // Text colour only. Goal 6's premise is that background graphics are OFF,
  // emulateMedia does not enable background painting, and page.pdf() defaults
  // printBackground:false. #111111 is --text-primary from print.css.
  for (const [what, loc] of [
    ['statement root', statement],
    ['realized figure', page.locator('[data-testid="figure-realized"]')],
  ]) {
    const color = await loc.evaluate((el) => getComputedStyle(el).color)
    if (color !== 'rgb(17, 17, 17)') fail(`${what} computed ${color}, expected rgb(17, 17, 17)`)
  }
  ok('palette inverted to dark text')

  const sidebar = await page.locator('.sidebar').evaluate((el) => getComputedStyle(el).display)
  if (sidebar !== 'none') fail(`sidebar computed ${sidebar}, expected none`)

  // Nothing printable inside the printed subtree. The component test cannot cover
  // this: it renders the component in isolation and never sees the Print control.
  const interactive = await page.locator(
    '[data-print="statement"] button, [data-print="statement"] input, '
    + '[data-print="statement"] select, [data-print="statement"] svg',
  ).count()
  if (interactive !== 0) fail(`${interactive} interactive elements inside the printed subtree`)
  ok('printed subtree holds no controls')

  // Measured ON THE STATEMENT, not the document. .main-content declares overflow-y:auto
  // (index.css:163), which makes its computed overflow-x auto too, so an over-wide table
  // is clipped at that container and at the page box instead of being reported by the
  // document -- a document-level check can never fail.
  //
  // Viewport set to the printed page box first, because emulateMedia({media:'print'})
  // changes the media type but NOT the layout viewport: at 1400px this could only fail
  // at a width nobody prints at. The shipped PDF lays out at the @page CONTENT box -- A4
  // at 210mm minus print.css:13's 14mm margins = 182mm = 688px at 96dpi. (794px is the
  // whole sheet; the browser lays no content into the margins.) print.css shrinks the
  // statement table's cells but overrides no container padding, so .main-content keeps the
  // 40px each side index.css:162 gives it and the statement's box is 688 - 80 = 608px. A
  // 688px viewport reproduces that 608px exactly, so the residual against what ships is
  // zero. Nine columns at the on-screen 16px cell padding need 288px of padding alone, which
  // is what print.css's .statement-table rule buys back -- see the arithmetic in the comment
  // there. (.main-content is overflow-y:auto, so a page tall enough to scroll would shave
  // ~15px off clientWidth -- stricter than the printed page, never more permissive.)
  //
  // If this fails on a real run it is a finding about the shipped PDF -- the printed
  // table really is too wide for the page -- not a broken assertion. Do not "fix" it by
  // moving the viewport back toward the desktop width.
  await page.setViewportSize({ width: 688, height: 1123 })
  const [sw, cw] = await statement.evaluate((el) => [el.scrollWidth, el.clientWidth])
  if (sw > cw) fail(`horizontal overflow at the printed page box: scrollWidth ${sw} > clientWidth ${cw}`)

  // A non-empty PDF. No page-count assertion: that needs a PDF parser and
  // frontend/package.json has none. preferCSSPageSize:true so the A4 @page rule is
  // what gets measured -- Playwright's defaults are Letter with false.
  const pdf = await page.pdf({ format: 'A4', preferCSSPageSize: true, printBackground: false })
  if (pdf.length === 0) fail('page.pdf() returned an empty buffer')
  ok(`pdf rendered (${pdf.length} bytes)`)
} catch (err) {
  fail(err.message)
  // A first-ever run failing on a locator timeout or a refused fetch tells you nothing
  // from the message alone. The stack names the frame that threw; printed separately so
  // the failure tally stays one line per failure.
  if (err.stack) console.error(err.stack)
} finally {
  // run.sh echoes `screenshot: $SMOKE_SHOT` on a non-zero exit, so this smoke has to
  // actually write that file or the message points at a path that does not exist.
  // In finally, not in the catch: a failing assertion calls fail() without throwing, so
  // the catch only sees the throwing half of the failures. Guarded by .catch so a
  // screenshot error cannot mask the real failure or move the exit code.
  if (failures.length && process.env.SMOKE_SHOT) {
    await page.screenshot({ path: process.env.SMOKE_SHOT, fullPage: true })
      .catch((e) => console.error(`screenshot not written: ${e.message}`))
  }
  await browser.close()
}

if (failures.length) { console.error(`${failures.length} failure(s)`); process.exit(1) }
console.log('statement smoke OK')
