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
  // Width is the other half of the fixture, and it is where the live provider would sneak in.
  // Only `realized` is deterministic here: it is replayed from the transactions below and never
  // read from a quote. Price, market value and unrealized all come from the live provider, and a
  // table's columns are interdependent, so a wide live-priced cell would make the body-cell
  // line-count check at the bottom of this file depend on what a provider quotes today -- which
  // the header of this file forbids. The fixture is shaped so that cannot happen:
  //
  //   realized      149970 x (5 - 120) = -$17,246,550.00. A leading-minus eight-figure value,
  //                 fully deterministic, so the table is never tested at its narrowest.
  //   market value  30 shares x live price. The 30,000-share version of this fixture measured
  //                 $15,994,799.19 in a probe; at 30 shares the cell is around $16,000, three
  //                 digits narrower, so a 10x move in the quote moves the cell by one character.
  //                 Price and unrealized shrink with it, and a provider outage (prices of 0)
  //                 makes every live column narrower still, never wider.
  //
  // That is the whole reason the sell is 149970 and not a round 120000: the leftover position is
  // 30 shares ON PURPOSE, and nobody should tidy it back to a round number.
  //
  // The Type column prints the display label `Fund` (PortfolioStatement.tsx), not the stored
  // MUTUAL_FUND, so the widest Type token this fixture prints is now `Stock`. Earlier versions of
  // this comment
  // recorded MUTUAL_FUND rendering whole in a 96px Type cell at 688px and shattered into 37px at
  // 608px, and a table max-content of 694px against the 688px page box. All of those were
  // measured with the raw enum, an eight-figure market value and the Type-column break rule, and
  // all three are gone, so no max-content figure is restated here. What has not changed is what
  // the two checks at the bottom are for: at the shipped width every body cell must render on one
  // line, and a table squeezed below what it needs must show up as a failure rather than as
  // smaller, broken type. If the line-count check fails on a real run that IS the finding: do not
  // widen the viewport and do not narrow the fixture.
  const holdings = [
    { symbol: 'AAPL', name: 'Apple Inc.', asset_type: 'STOCK', txs: [
      ['BUY', '60', '100', '2024-01-01T00:00:00Z'],
      ['SELL', '20', '130', '2024-12-01T00:00:00Z'],
    ] },
    { symbol: 'MSFT', name: 'Microsoft', asset_type: 'MUTUAL_FUND', txs: [
      ['BUY', '150000', '120', '2024-01-01T00:00:00Z'],
      ['SELL', '149970', '5', '2024-12-01T00:00:00Z'],
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

  // Same argument again, for the rule that would be visible on screen if it escaped: print.css
  // collapses .main-content's 40px side padding (index.css:162) when the statement is what is
  // printing, and that rule belongs inside @media print. Move it outside and the statement tab
  // loses its padding on screen while PrintCss.test.ts stays green -- it pins the rule as text
  // and cannot see @media membership at all. Nothing else in any gate would notice.
  const mainPaddingLeft = await page.locator('.main-content')
    .evaluate((el) => getComputedStyle(el).paddingLeft)
  if (mainPaddingLeft !== '40px') {
    fail(`.main-content computes paddingLeft ${mainPaddingLeft} on screen, expected 40px: the statement print padding collapse escaped @media print`)
  } else {
    ok('.main-content keeps its 40px on-screen padding, so the print collapse is inside @media print')
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
  // whole sheet; the browser lays no content into the margins.) print.css collapses
  // .main-content's 40px side padding when the statement is what is printing, so the
  // statement's box IS that 688px content box and a 688px viewport reproduces it exactly --
  // residual zero. (Before that rule the box was 688 - 80 = 608px and the same viewport
  // reproduced it exactly: the box moved, the viewport did not need to.) Nine columns at the
  // on-screen 16px cell padding need 288px of padding alone, which is what print.css's
  // .statement-table rule buys back -- see the arithmetic in the comment there. (.main-content
  // is overflow-y:auto, so a page tall enough to scroll would shave ~15px off clientWidth --
  // stricter than the printed page, never more permissive.)
  //
  // What this CAN see: a table whose min-content exceeds the box -- a shape whose unbreakable
  // content is genuinely wider than the page, which is the silent clip described above. What it
  // cannot see on its own is a table that is merely squeezed -- forced below max-content while its
  // min-content still fits. print.css used to carry
  // carry `overflow-wrap: anywhere` on the Type column, which let MUTUAL_FUND shatter into a
  // fragment small enough to hold min-content under the box -- measured at 608px scrollWidth with
  // a 37px Type cell, while the same fixture at 688px measured 694px of max-content and a 96px
  // Type cell with the token whole. That break rule is gone (see print.css) and the Type column
  // prints a display label now, so a squeeze has nothing left to shatter. Whether it now surfaces
  // here, in the line-count check below, or in both is a browser measurement to take again, not
  // something to assert from here; no max-content figure is restated for the same reason. Both
  // checks stay, because between them a squeezed or over-wide table cannot pass quietly.
  //
  // If this fails on a real run it is a finding about the shipped PDF -- the printed
  // table really is too wide for the page -- not a broken assertion. Do not "fix" it by
  // moving the viewport back toward the desktop width.
  await page.setViewportSize({ width: 688, height: 1123 })
  const [sw, cw] = await statement.evaluate((el) => [el.scrollWidth, el.clientWidth])
  if (sw > cw) fail(`horizontal overflow at the printed page box: scrollWidth ${sw} > clientWidth ${cw}`)

  // No body cell may wrap onto a second line. This check and the overflow check above are the only
  // assertions anywhere that can see print.css's
  // `.main-content:has([data-print="statement"]) { padding: 0 0 24px }` being DELETED (the
  // on-screen check earlier only sees it escaping @media print). Which of the two a deletion trips
  // now is an open browser question: the measurement that made THIS one the discriminator (Type
  // column 96px -> 37px, MUTUAL_FUND broken across two lines) belonged to the raw enum and to the
  // Type-column break rule, and both are gone.
  //
  // Element height cannot answer this. Cells in a row share the row's height, so in a wrapped
  // row every td in it measures the row's height and none of them discriminates. A Range over a
  // cell's contents returns one client rectangle per line box that text actually occupies --
  // per cell, independent of the row -- so its rectangle count IS the line count.
  //
  // ALL body cells, the live-priced ones included, on purpose. Narrowing this to the deterministic
  // columns would be the weaker assertion for no gain, because seed() keeps the live-priced
  // columns narrow -- 30 shares left, so market value is around $16,000 instead of the eight
  // figures an earlier fixture carried -- precisely so that a wrap here is a layout finding and not
  // a report of what a provider quoted today. A wrapped header is acceptable -- "Unrealized ROI"
  // is two words -- so the check stays scoped to tbody, where a wrap means the squeeze has reached
  // the figures themselves.
  //
  // The failure names EVERY offender, not just the first: the columns are interdependent, so one
  // wrapped cell and five wrapped cells are different diagnoses.
  //
  // No empty-list guard: the figure-realized colour check above throws if the populated branch
  // did not render at all, so there is no path to this check seeing zero body cells and passing.
  const cells = await page.locator('[data-print="statement"] tbody td').evaluateAll((tds) =>
    tds.map((td, i) => {
      const r = document.createRange()
      r.selectNodeContents(td)
      return { i, text: td.textContent ?? '', lines: r.getClientRects().length }
    }),
  )
  const wrapped = cells.filter((c) => c.lines > 1)
  if (wrapped.length) {
    fail(`${wrapped.length} statement body cell(s) render across more than one line: `
      + wrapped.map((c) => `#${c.i} ${JSON.stringify(c.text)} (${c.lines} lines)`).join(', ')
      + ' -- the table has been squeezed below what it needs, which the overflow check above cannot see')
  } else {
    ok(`no statement body cell wraps to more than one line (worst cell ${Math.max(1, ...cells.map((c) => c.lines))} line(s) across ${cells.length} cells)`)
  }

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
