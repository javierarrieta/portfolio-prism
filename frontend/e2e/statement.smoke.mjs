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
  // Two USD holdings in a USD portfolio. Same-currency is a hard requirement: a
  // foreign-currency asset makes tax-summary 500, the guard renders nothing, and
  // the smoke would print the dashboard and still pass.
  for (const [symbol, name] of [['AAPL', 'Apple Inc.'], ['MSFT', 'Microsoft']]) {
    const asset = await api(`/portfolios/${portfolio.id}/assets`, {
      method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ symbol, name, asset_type: 'STOCK', sector: 'Technology', currency: 'USD', isin: '' }),
    })
    for (const [type, quantity, price, date] of [
      ['BUY', '60', '100', '2024-01-01T00:00:00Z'],
      ['SELL', '20', '130', '2024-12-01T00:00:00Z'],
    ]) {
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
  // cannot see @media membership -- a hide rule duplicated outside the block passes it
  // 5/5 while blanking the sidebar on screen. This is the assertion that catches that,
  // and it is only meaningful while the media type is still screen.
  const onScreen = await page.locator('.sidebar').boundingBox()
  const onScreenDisplay = await page.locator('.sidebar').evaluate((el) => getComputedStyle(el).display)
  if (!onScreen || onScreen.width === 0 || onScreen.height === 0 || onScreenDisplay === 'none') {
    fail(`sidebar not visible on screen (box=${JSON.stringify(onScreen)} display=${onScreenDisplay}): a hide rule escaped @media print`)
  } else {
    ok('sidebar visible on screen, so the hide rules are inside @media print')
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

  // Measured ON THE STATEMENT, not the document. index.css:47 sets
  // body{overflow-x:hidden}, which propagates to the viewport, so a document-level
  // check can never fail -- it clips silently.
  //
  // Viewport set to the printed page box first, because emulateMedia({media:'print'})
  // changes the media type but NOT the layout viewport: at 1400px this could only fail
  // at a width nobody prints at. The shipped PDF lays out at the @page CONTENT box -- A4
  // at 210mm minus print.css:13's 14mm margins = 182mm = 688px at 96dpi. (794px is the
  // whole sheet; the browser lays no content into the margins.) print.css overrides no
  // padding anywhere, so .main-content keeps the 40px each side index.css:161 gives it
  // and the statement's box is 688 - 80 = 608px. A 688px viewport reproduces that 608px
  // exactly, so the residual against what ships is zero. Nine columns of 16px cell
  // padding is ~288px of padding alone, which is why the ~106px between 608 and 714 was
  // the whole point of the check. (.main-content is overflow-y:auto, so a page tall
  // enough to scroll would shave ~15px off clientWidth -- stricter than the printed
  // page, never more permissive.)
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
