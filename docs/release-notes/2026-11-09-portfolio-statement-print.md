# Printable portfolio statement: a page you can hand to an adviser

Branch `feat/report-export`.

## What a user will notice

**A fifth tab, Printable Statement.** It renders the `tax-summary` payload the app already
fetches. No new route, no new field, no new figure on any existing endpoint, no OpenAPI
regeneration — the branch touches no backend file. The holdings table carries the same
figures the holdings breakdown already shows, without its expand column or its Actions
column.

**Realized prints as cumulative since inception, unrealized as of today, and the page says
which is which.** The labels are literal: `Realized P&L — cumulative since inception` and
`Unrealized P&L — as of today`. The tax-summary endpoint has no date window, so there is no
quarterly realized figure available to print, and an unlabelled realized number next to a
quarterly tax form is how a wrong filing starts.

**The page states that it rounded.** `Figures are rounded for display from full-precision
calculations.` The engine emits full-precision decimals and the page prints whatever
`formatters.ts` renders for the currency — 2 dp for USD and EUR, **0 dp for a zero-decimal
currency** (`1234.565` prints `¥1,235` for JPY), up to 8 dp for BTC/ETH, and exactly 2 dp
for any other code — an unrecognised one falls back to the same 2 dp rendering rather than
an error. The reconciliation this disclosure exists for: the only CSV export in the app
today, the sell P&L breakdown (`sellPnlCsv.ts`), is full-precision by standing decision, so
a reader who reconciles a printed realized figure against that export is comparing the same
underlying numbers at two different precisions and the last digits will not match. One
number, two renderings, and without the note that reads as a bug. The tax report and its CSV
are specified and not built; when they ship, the rounding line gains that sentence in that
change.

**An empty portfolio prints a note, not three zeros.** The heading, the printed date, and
`No holdings in this portfolio on Nov 9, 2026.` — no figures, no run context, no caveats.
Three zeros under a "Portfolio Statement" heading would misrepresent the state, and a
splits caveat under "no holdings" is nonsense.

**The run context names the strategy.** `{portfolio} · {currency} · {strategy}`, with
`(threshold Nd)` appended only for `HYBRID`, because `threshold_days` means nothing to FIFO
and LIFO. The claim it can make is "the strategy the app sent and the server echoed", not
"the strategy the engine used" — this route defaults the strategy but does not normalize it.

**Printing from the other four tabs is unchanged — except the paper.** `@page` takes no
selector, so a print started from the dashboard, analytics or ledger now gets A4 with 14 mm
margins instead of the browser default. Everything else about those prints — sidebar, nav,
charts — is exactly as it was.

## Behaviour changes worth knowing about

- **Two dates on the page, and they can disagree.** `Printed Nov 9, 2026` is the client
  render date. `Data fetched Nov 9, 2026, 10:30 AM` is when the tax-summary request landed.
  The statement issues no request when you open it, so a page printed the morning after a
  session opened yesterday carries today's date over yesterday's prices. `Data fetched` is
  the only claim on the page about when the numbers were true.
- **A late-recorded split changes past figures, and the page says so.** Splits are
  auto-detected while the tax summary is computed (`analytics.rs:63-65` calls
  `sync_splits_for_asset` per asset before it replays lots), at most once per symbol per
  30 days, and a split dated before the asset's first transaction is skipped. A split that
  lands late — from that sync or entered by hand — is applied to the whole replay, so
  figures on a page you printed last month can move. Hence
  `Split-adjusted history is as recorded when this page was loaded; late-recorded splits
  can change past figures.` The sync-in-a-read-path shape itself is issue #50.
- **If the tax-summary request failed, the tab renders nothing.** The statement renders only
  when there is a summary for the portfolio currently selected. With none, nothing in the
  print stylesheet engages — every hide rule and the palette inversion are scoped with
  `:has([data-print="statement"])` — so a print started in that state gets the app shell as
  it looks on screen (sidebar, header, the error banner) with an empty content column, in the
  dark palette, which is to say illegible. That is the absence of a designed state rather than
  a new failure mode, and it is bounded: the same failed request leaves the dashboard on its
  "No Active Assets Found" card (`Dashboard.tsx:26`), so nothing here is worse than the state
  the app already has. On screen the error banner is visible, so the failure is not silent
  where you are looking at it.
- **Switching portfolios cannot print the wrong portfolio's numbers.** The summary is
  guarded against the selected portfolio id (`taxMeta` in `App.tsx`). Between switching and
  the new response landing, the statement renders nothing rather than portfolio B's figures
  under portfolio A's name — which is the one failure this artifact cannot absorb quietly,
  because it looks correct.
- **There is no URL for it.** The app has no router, so the statement is a tab and cannot be
  bookmarked or linked. An adviser has no account and cannot log in. The artifact therefore
  necessarily leaves the app — print, or print to PDF, then forward — which is why the page
  carries its own provenance block instead of relying on the app around it.

## What is verified, and what is not

**Covered by `npm test` (20 files, 173 tests, green as of this note):**

- `PortfolioStatement.test.tsx` — the structural hooks, the literal copy, the nine table
  columns, the empty branch's negative list, and one rounding golden (`1234.565` at EUR
  renders `€1,234.57`). jsdom does not apply `@media print`, so this proves structure and
  formatting and nothing about the printed page.
- `PrintCss.test.ts` — `print.css` read **as text**: both hide rules scoped with `:has()`,
  the inverted palette values, the `.statement-table td` border override, the break rules,
  and `@page` A4/14 mm. Text, not rendering — it cannot tell you the rules apply to the
  elements they name.
- `AppStatement.test.tsx` — the tab is reachable by the label a person can actually see; the
  statement root and the Print button are direct children of `.main-content`, which is the
  position the hide rules need (a wrapper there prints a blank page from a tab that looks
  right on screen); nothing renders while the loaded summary belongs to another portfolio.

**Written but never executed.** `frontend/e2e/statement.smoke.mjs` (`npm run e2e:statement`,
prerequisites in `frontend/e2e/README.md`) is the only test in the repository that sees real
print CSS: it emulates print media, checks the sidebar computes `display: none`, checks the
text colour is `rgb(17, 17, 17)`, checks no control sits inside the printed subtree, checks
for horizontal overflow at the A4 content box, and checks `page.pdf()` returns a non-empty
buffer. **It has never been run.** The
workspace that produced this change has no chromium and none was installed. It is
syntax-checked and reviewed, which is not the same as executed, and the first real run should
be read as a first run rather than a regression. When it does run it asserts **no figure** on
purpose: prices come from a live provider, and a smoke that asserted figures would print
zeros and still pass.

**There is no CI browser job.** Issue #45 is open. One is possible — CI could install chromium,
and `emulateMedia` and `page.pdf()` need no display server — but the job is not browser-only:
`e2e/run.sh` compiles and boots the Rust API and Vite, so it costs a full cargo build. A
`continue-on-error` job nobody reads would read as a pass. The useful form is a blocking job,
and that is #45's decision, not this slice's.

**So page count, A4-on-Letter behaviour, and legibility on paper have exactly one check each:
a human with a print preview.** Nothing automated counts pages (a page-count assertion needs
a PDF parser, and `frontend/package.json` has none), and nothing in this repository has ever
put this page on paper. The walkthrough is `docs/auth-testing.md` §5, *Printing the
statement*. Treat it as a required step before relying on the feature, not a formality —
this slice deliberately ships the print path unverified by CI.

## Not changed here

- No backend change of any kind: no new route, no new field, no new figure, no OpenAPI
  regeneration.
- No print stylesheet for the dashboard, analytics or ledger tabs. Printing from them still
  prints the sidebar, the nav and a Recharts SVG — now on A4.
- No date parameter and no past-period statement. Today-only by decision; the dated artifact
  is the tax report, which is specified and not built.
- No server-side PDF and no PDF dependency. The browser prints.
- No router, no deep link, no shareable statement URL.
- Split synchronisation still runs inside read paths (#50) and is still throttled per symbol
  (#38).
