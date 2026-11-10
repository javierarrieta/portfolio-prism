# Portfolio Statement (Printable)

*   **Author:** javierarrieta, with EGM review support
*   **Status:** Proposed — reviewed and **implementation-ready**. Five stages of EGM review are
    complete. Stages 5–8 findings are applied; of Stage 9's five blockers, four are closed (prop
    signature, structural hooks, nav item, rounding-rule contradiction) and the fifth — the
    null-payload state — is closed as an **accepted gap**; see *When there is no payload at all*.
    Sixteen Stage 9 majors remain recorded in
    `docs/superpowers/reviews/2026-11-09-statement-print/consolidated_review_report.md`; the ones that
    blocked implementation are closed, the rest are citation drift and restatement cleanup.
*   **Date:** 2026-11-09
*   **Inherits from:** `2026-11-09-pricing-layer-truthfulness.md` — the statement renders the same
    FX conversion that document describes as broken for foreign-currency assets. It **inherits that
    pre-existing defect**; it is not blocked on the fix, because same-currency assets involve no FX at
    all, and a foreign-currency portfolio already fails the `tax-summary` request outright today
    (`currency_service.rs:234` → `analytics.rs:86` returns 500) — so nothing this slice prints can be
    worse than the dashboard's current behaviour. See *When there is no payload at all*.
    **Related:** `2026-11-09-tax-report-and-statement-design.md` (the tax report; this document was
    split out of it).
*   **Origin:** split out of the tax-report design during its review; the scope judgement that
    split it is stage 8 of `docs/superpowers/reviews/2026-11-09-tax-report/zoom_out_2_report.md`.
*   **Naming note:** below, **"the parent design" always means the tax-report design**, not the
    pricing-layer document.
*   **Review record:** derivations moved out of this spec — precision forensics, e2e harness
    forensics, correction history — are in
    `docs/superpowers/reviews/2026-11-09-statement-print/decision_records.md`.

## Overview

A printable page for one portfolio: realized P&L cumulative from inception, unrealized P&L as of
today, and the holdings behind them. The owner reads it or hands it to an adviser. The browser
prints it to PDF; the server never produces a PDF.

It takes no date. The tax report is the dated artifact; a statement about a past quarter is not on
the roadmap this document covers. Because it is always today, **it needs no backend change at all**:
it renders the `tax-summary` payload the app already fetches.

That is the appeal of this slice and also its limit. The hard parts are printing a dark-themed app
legibly, and making a page whose figures are rounded say so rather than pretend to be the exact
file next door.

## The Problem

*   **There is no printable view.** `grep -r "@media print" frontend/src` returns nothing. The
    owner's only option today is Ctrl-P on the dashboard, which prints the sidebar, the navigation,
    the error banner and a Recharts SVG.
*   **The app is dark glassmorphism, and dark backgrounds are not printed by default.**
    `index.css:6-30` sets `--bg-dark: #090d16` and `--text-primary: #f8fafc`, and `.glass-card`
    uses `backdrop-filter: blur(16px)`. Browsers omit background colors unless the user turns on
    background graphics, so a naive print is near-white text on white paper — invisible, not merely
    ugly. The print stylesheet's real job is **palette inversion**; hiding the nav is the easy part.
*   **The printed page must say what it rounded.** The backend sends money as full-precision decimal
    strings (`db_types.rs:31`) and the engine multiplies by the FX rate with no scale control
    (`tax_engine.rs`, the FX multiplications in `replay`), so a realized figure can carry more
    decimals than a page should show, while the frontend rounds at display. A printed figure and the
    tax report CSV's figure for the same holding therefore show **different digits** — by design, one
    rounded and the other not — and are the same number once both are rounded identically. A page
    printing `1,234.57` beside a CSV cell reading `1234.56500000000000000000`, while claiming to be
    the same number, is the defect. The fix is to say the rounding out loud on the page.
*   **There is no router, so the statement is not linkable.** `App.tsx:36` is
    `useState(DEFAULT_TAB)`; the nav is four `<li className="nav-item">` elements
    (`App.tsx:386-398`). A printed page cannot be re-opened by URL or bookmarked, and this slice
    does not solve that. Rejected: Alternative G.
*   **Nothing can prove the print path works.** `.github/workflows/ci.yml` has no browser job and
    issue #45 (*Run the sell P&L Playwright smoke in CI*) is open, so **the print path ships
    unverified**: the component test verifies structure and rounding, but no automated check renders
    the page under print media. The smoke is a local step for reasons recorded in the review record;
    it is a choice about CI cost, not an impossibility. Selectors and smoke assertions are named
    here rather than left to implementation time, so the check is specific the moment #45 lands.
*   **The person the statement is for cannot log in.** Every `/api` route takes `AuthedUser` plus a
    `require*` call, and `apiFetch.ts:22-26` sends an unauthenticated caller to `/auth/login`. An
    adviser has no account. The artifact necessarily leaves the app: the owner prints, then forwards.
    So the page must stand on its own once detached, which is what Goal 4's provenance block and
    Goal 3's declared rounding are for. Rejected: Alternative F.

## Goals

*   **Goal 1 — the P&L figures, labelled.** Cumulative realized P&L and unrealized P&L for the
    selected portfolio, with realized labelled **cumulative since inception**; portfolio value prints
    alongside and needs no such label. The undated `tax-summary` endpoint has no window, so a
    statement cannot show a quarterly realized figure, and an unlabelled number next to a quarterly
    tax form is how a wrong filing starts.
*   **Goal 2 — the printed page is a document.** One column. No sidebar, navigation, buttons,
    inputs, `select` elements, charts or SVG, error banner, or horizontal overflow at A4 width.
    `@page { size: A4; margin: 14mm }`, with the caveat from *Palette inversion* that `@page` cannot
    be scoped and so also changes the paper size of prints started from other tabs.
*   **Goal 3 — the printed numbers are the payload's numbers, rounded once and declared.** The
    statement is a **display artifact**. Every money value is a payload value rendered through
    `frontend/src/utils/formatters.ts` and rounded at display, nowhere else. The rule is "whatever
    `formatters.ts` renders", not "to the cent": `EUR` renders 2 dp, `JPY` renders **0 dp**
    (`1234.565` → `¥1,235`), `BTC`/`ETH` up to 8 (`formatters.ts:10-11`), and any other crypto code
    falls to the `catch` at `formatters.ts:19-22` at exactly 2 dp. Every other screen follows the
    same rule. The statement performs no arithmetic: it sums nothing, differences nothing, never
    re-derives a total. The engine's output is full precision, so the page prints a visible line
    saying its figures are rounded from that precision. When the tax report ships, its cumulative
    realized column should equal the statement's figure rounded the same way `formatters.ts` rounds
    it; that cross-check is conditional on the tax report existing.
*   **Goal 4 — the page states its own provenance.** Portfolio name and base currency from the
    portfolio object; strategy and `threshold_days` as the server echoed them (`analytics.rs`, the tail
    of the tax-summary response); the printed date, the data timestamp, and the splits caveat — each
    with its hook and literal copy in *On-page copy*. See *Money* for the currency rule. `threshold_days` prints only for `HYBRID`, because it is unused
    by `FIFO` and `LIFO` (`tax_engine.rs:645-649`). The empty-portfolio branch (`analytics.rs:45-51`)
    echoes none of the three fields, so the run-context line is omitted there rather than rendered
    from defaults; a line reading "realized · (threshold 0d)" would be a fabricated run context. See
    *What "splits" and the strategy line mean* for both terms and for the narrower claim this slice
    can make about the echoed strategy.
*   **Goal 5 — no backend change in this slice.** No new route, no new field, no new figure on any
    existing endpoint, no OpenAPI regeneration. This is a **sequencing** boundary, not a standing
    principle: the parent design will rewrite this handler's response and add `from`/`to`. The
    handler currently returns `Json<serde_json::Value>`, which is why the committed
    `docs/openapi/openapi.json` gives this response an empty schema (`:326-334`). If implementation
    requires a backend change, stop and re-scope. The boundary's cost: the run context printed here
    is a subset of what the parent design will carry anyway. It is kept because any response-shape
    change lands in a file CI diffs byte-for-byte, so it is not a quiet two-field change.
*   **Goal 6 — the palette inversion does not depend on a browser setting.** It must not rely on
    "background graphics" being ticked: text becomes dark because the stylesheet says so, not
    because the browser kept a background. The second half of the requirement is the case the
    setting *is* on — then the background overrides must also be present, or that reader gets dark
    text on the dark app background. *Palette inversion* step 4 says which variables carry which
    half.
*   **Goal 7 — the empty portfolio says so.** `analytics.rs:45-51` returns three zeros and no
    holdings. Printing three zeros under a "Portfolio Statement" heading misrepresents the state, so
    that branch prints the heading, the printed date and the empty note, and nothing else. See
    *Empty portfolio* for the branch and the negative list.

## Non-Goals

*   **No date parameter, and no past-period statement.** Decided in the parent design.
*   **No server-side PDF, and no PDF dependency.** See Alternative A.
*   **No print-CSS gate in CI.** See Alternative B and Risks.
*   **No router, no deep link, no shareable statement URL.**
*   **No per-asset price-date provenance on the statement.** The undated endpoint returns no
    `price_date_used`, so the page states only what it can know: the portfolio, the strategy, the
    printed date and the data timestamp. Per-asset pricing provenance belongs to the tax report,
    which does not exist yet.
*   **No cross-portfolio statement.** One portfolio, as in the tax report.
*   **No print theme picker, no "dark print" mode.**
*   **No print for the other tabs.** See *Other tabs*.

## Technical Plan

### Surface: a fifth tab, not a route

`statement` joins `dashboard`, `portfolios`, `analytics` and `ledger` in the existing nav. It renders
from the `taxSummary` state that `fetchPortfolioData` already populates (`App.tsx:114`), so opening
the statement issues no request of its own.

The fifth nav item is specified completely, because it is the smoke's only entry point and there is
no tab union to extend — `DEFAULT_TAB` is a bare `const` (`App.tsx:31`) and `useState(DEFAULT_TAB)`
infers `string` (`:36`), so the nav-side work is one `<li>` and one branch:

```jsx
<li className={`nav-item ${activeTab === 'statement' ? 'active' : ''}`} onClick={() => selectTab('statement')}>
  <Printer size={18} /> Printable Statement
</li>
```

`Printer` is a new import from `lucide-react`, joining the twelve already imported at `App.tsx:2-13`.
The label follows the existing pattern of a multi-word visible name unrelated to the tab id
(`Holdings & Tax lots`, `Risk & Correlation`). The smoke navigates with
`page.getByText('Printable Statement').click()`, which is the working idiom
(`sell-pnl.smoke.mjs:107`); "accessible name" would be wrong, since these are `<li onClick>` with no
role.

The Print control is a button **outside** the printed subtree, calling `window.print()`, positioned
as a sibling of the statement root so the hide rules remove it — see *Palette inversion*, rule 2.
Its copy is `Print` and it carries `data-testid="print-button"`; it is not printed, so it needs no
copy-table row.

### The component's interface

Target Files 1 says "takes the summary as a prop"; that is not enough, because the required copy needs
three inputs and the payload carries only one of them cleanly. Stated verbatim:

```ts
interface PortfolioStatementProps {
  taxSummary: TaxSummary;   // the figures and the money currency
  portfolio: Portfolio;     // name and base currency for the header and run-context
  fetchedAt: Date;          // the data-at line; see the two-clock rule below
}
```

`portfolio` is the whole object rather than two strings, matching how the sibling tab is rendered
(`App.tsx:485` passes `portfolio={...}` to `PortfolioDetail`). `Portfolio` already has `name` and
`currency` (`types.ts`), so `run-context`'s portfolio name and the header's base currency both come
from it — the payload carries no portfolio name, which is why the two new optional `TaxSummary`
fields alone are not sufficient.

"Takes no date parameter" means no **user-selected** date; `fetchedAt` is a captured timestamp, not an
input the user chooses.

App-side state, all three set at the one place `setTaxSummary` is called (`App.tsx:114`):

```ts
const [taxSummary, setTaxSummary] = useState<TaxSummary | null>(null);   // unchanged; three tabs read it
const [taxMeta, setTaxMeta] = useState<{ portfolioId: number; fetchedAt: Date } | null>(null);
```

`taxMeta` is the parallel id from *Target Files 4*, kept separate so `Dashboard` and `PortfolioDetail`
keep reading `taxSummary` untouched. The branch renders only when
`taxSummary && taxMeta?.portfolioId === selectedId`.

### Structural hooks and the holdings table

The copy table fixes strings; these fix the elements the assertions and the CSS name. Without them,
four specified assertions cannot be written and print rule 5 has no selector.

| Hook / class | Element | Used by |
| :-- | :-- | :-- |
| `data-print="statement"` | the statement root `<section>` | print rules 1–2; layer 1 asserts its presence |
| `className="statement-headline"` | the three-figure block | print rule 5 ("never splits") |
| `data-testid="figure-value"` | portfolio value | layer 1 presence; layer 2 colour |
| `data-testid="figure-realized"` | cumulative realized P&L | layer 1 presence; layer 2 colour |
| `data-testid="figure-unrealized"` | unrealized P&L | layer 1 presence |
| `data-testid="holdings"` | the holdings `<table>` | layer 1 column and row assertions |

The table is `<table className="data-table statement-table">` — `.data-table` for the existing
typography, `.statement-table` as the print hook. Nine columns, in this order, with the renderer
named per column so no one has to guess which of the repo's three number formatters applies:

| `<th>` | Renderer |
| :-- | :-- |
| `Symbol` | plain text |
| `Type` | plain text — **not** the `.badge` markup `PortfolioDetail.tsx:272-276` uses, whose background is not a variable and would not invert |
| `Quantity` | `toLocaleString(undefined, { maximumFractionDigits: 6 })`, as `PortfolioDetail.tsx:277` does |
| `Avg cost`, `Price`, `Market value`, `Realized`, `Unrealized` | `formatCurrency(v, taxSummary.currency)` |
| `Unrealized ROI` | `formatPercent(v)`, as `PortfolioDetail.tsx:292-294` does |

No success/danger colour on the P&L cells — sign is in the number, per *Palette inversion* step 4.
Layer 1 asserts the nine header strings and one row per asset; the golden below pins the formatting.

### On-page copy, written out

Seven strings are requirements rather than implementation detail. Goals 1, 3, 4 and 7 assert six of
them; `data-at` exists because of the two-clock problem described below, not because a Goal names it.
Each is written here with a stable hook, because an assertion that "the rounding line is present" is
not an assertion until the string or the hook is fixed. Copy is English, matching the rest of the
app's UI, and assertions match `textContent`, so these are literal text and not markup: write `P&L`,
not `P&amp;L`.

Hooks use the repo's existing `data-testid` convention (`SellPnlModal.tsx:209`,
`sell-pnl.smoke.mjs` `getByTestId('strategy-note')`). The one exception is `data-print="statement"`,
which is load-bearing in CSS and asserted once in the component test.

| Hook | Copy | Printed on the empty branch? |
| :-- | :-- | :-- |
| `data-testid="label-realized"` | `Realized P&L — cumulative since inception` | no |
| `data-testid="generated-at"` | `Printed Nov 9, 2026` — `printDate` is `new Date().toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric' })`, the idiom already at `Dashboard.tsx:114` | yes |
| `data-testid="data-at"` | `Data fetched Nov 9, 2026, 10:30 AM` — `fetchedAt.toLocaleString('en-US', { month: 'short', day: 'numeric', year: 'numeric', hour: '2-digit', minute: '2-digit' })`, the date idiom above extended to the minute; no such idiom exists in the repo yet, so this slice establishes it | no |
| `data-testid="rounding-note"` | `Figures are rounded for display from full-precision calculations.` | no |
| `data-testid="splits-note"` | `Split-adjusted history is as recorded when this page was loaded; late-recorded splits can change past figures.` | no |
| `data-testid="empty-note"` | `No holdings in this portfolio on Nov 9, 2026.` | yes, alone |
| `data-testid="run-context"` | `{portfolio name} · {base currency} · {strategy}`, and when strategy is `HYBRID` a trailing ` (threshold {threshold_days}d)` — the leading space is part of the appended segment, not of the base string | no |

Layer 1 freezes the clock — `vi.useFakeTimers(); vi.setSystemTime(new Date('2026-11-09T10:30:00'))` —
so the three templated rows are assertable as literals. `test/setup.ts:3` pins `process.env.TZ =
'UTC'`, which is what makes a frozen clock deterministic rather than merely reproducible.

Three rules that fall out of the table:

*   **The rounding note is not conditional on the currency.** On any branch that prints figures, it
    prints for every currency, including one whose `Intl` rendering is already exact for the values
    in hand, because the engine's precision is not the currency's precision. Computing when it would
    be redundant is not worth the branch. It is **not** printed on the empty branch — see *Empty
    portfolio*.
*   **Two dates, two clocks, and the page says which is which.** The statement renders cached state
    and issues no request on open, so a page printed the morning after a session opened yesterday
    carries today's date over yesterday's prices. `generated-at` is the client render date; `data-at`
    is captured in `fetchPortfolioData` beside `setTaxSummary` and is the only claim on the page about
    when the numbers were true. The splits note therefore says "when this page was loaded" and not a
    date: the split rows were as recorded when the **request** ran, which is a server fact a client
    clock cannot assert. This is the largest provenance hole the rounding declaration does not cover,
    and it is why `data-at` exists even though the payload carries no date field of its own.
    `Dashboard.tsx:114` already prints a client-clock "Data as of" line, so this tightens an existing
    convention rather than inventing one.
*   **The copy never names an artifact that does not exist.** The tax report and its CSV are
    specified in a sibling design and are not built; a printed page an adviser may ask about cannot
    reference a file the owner cannot produce. When the tax report ships, the rounding note gains the
    CSV sentence in that change, not this one.

### Empty portfolio

`analytics.rs:45-51` returns `assets: []`, three zero strings and **no** `currency`, `strategy` or
`threshold_days`. Goal 7 is that branch: the component checks `taxSummary.assets.length === 0` and
renders the portfolio heading, `generated-at` and `empty-note` — and nothing else. No headline
figures, no `data-at`, no run-context, no splits note, no rounding note, no holdings table: a page
reading "No holdings in this portfolio" followed by "Split-adjusted history is as recorded…" is
nonsense. The existing holdings table already makes the same branch for the same reason
(`PortfolioDetail.tsx:238`), so this is a repo convention.

### When there is no payload at all — an accepted gap

There is a third state besides populated and `assets: []`: `taxSummary === null`. It is reached
whenever the `tax-summary` request fails, and for a foreign-currency portfolio it fails today —
`currency_service.rs:234` builds the malformed pair `EUR=USD`, `get_rate` returns `Err`, `replay`
propagates it (`tax_engine.rs:403`), and `analytics.rs:86` turns it into a 500. `App.tsx:112` throws,
`setTaxSummary` never runs, and the guard the mandated pattern produces renders nothing.

**The decision is to accept this and say so, not to design around it.** In that state the statement
tab renders nothing, so the `:has([data-print="statement"])` hide rules and the grid collapse
(`print.css:25-29`) do not match, and Ctrl-P prints the app shell — sidebar, header, error banner
— around an empty content column. It is **not** the dashboard: every tab branch in `App.tsx` is
gated on the active tab, so the content column holds only its header row. It is not illegible
either: the palette overrides (`print.css:36-49`) carry no selector, so that shell prints
dark-on-white. The gap is therefore not a new failure mode but the absence of a designed one, and
it is bounded: the same failed request leaves the dashboard on its "No Active Assets Found" card
(`Dashboard.tsx:26`), and the pricing-layer change removes the cause for
every foreign-currency holding. What this slice will not do is invent a fourth state, a copy string
for it, and a test for it, to paper over a defect another document already owns.

Two consequences are stated rather than left to be discovered:

*   **The banner that would explain the blank tab is printed, not hidden.** It lives inside
    `.header-row` (`App.tsx:458`), which print rule 2 hides — but rule 2 is `:has()`-scoped and
    does not match in this state, so the banner is on the printed page. On screen it is visible
    too, so the failure is not silent either way.
*   **The smoke's fixture must be same-currency.** Seeded holdings are denominated in the portfolio's
    base currency, or the harness 500s before any print assertion runs. That is a constraint on the
    fixture, not a thing under test.

### Money: a display artifact, declared

The statement renders payload numbers through `formatters.ts`, which rounds at display to whatever
`Intl` decides for that currency code — the rule stated normatively in Goal 3 and not restated here.
It does not attempt to be exact, and it says so on the page.

*   **Precision is lost before the component runs, not in the formatter.** `normalizeTaxSummary`
    (`decimal.ts:64-66` and `:71-77`) runs every money field through `num()`, which is `Number(v)`, and
    `TaxSummary` types them as `number` (`types.ts:89-95`). The full-precision string never reaches
    any statement code. The consequence for this spec: Goal 3's "the payload carries full precision"
    describes the wire, and the page's declaration is a statement about the engine's output, not
    about a value the browser could recover. Preserving decimal strings for the statement alone would
    mean a second payload shape that `formatCurrency` coerces back to a number anyway — Alternative
    C1, and `decision_records.md` §1 for the derivation.
*   **Why not make the page exact?** Exactness is the filed artifact's job; a page would print
    `1234.56500000000000000000` to a person. See Alternative C2.
*   **The reachable rounding case, and the one this spec does not claim.** A value with a third
    decimal place is reachable from the engine and display rounding moves it: `1234.565` renders
    `1,234.57` at 2 dp. Verification layer 1 pins it with a golden. Whether a *money* field can exceed
    double precision is unmeasured — the only measured 28-digit value on record is `roi`, a ratio — so
    this spec claims no numeric bound (`decision_records.md` §2).
*   **What the payload supplies.** Headline figures: `total_portfolio_value`, `total_realized_pnl`,
    `total_unrealized_pnl`. Run context: `strategy`, `threshold_days`, `currency`, all echoed by the
    server. Holdings rows come from `assets[]`: symbol, asset type, quantity, average cost, current
    price, market value, realized P&L, unrealized P&L, unrealized ROI — the same set
    `PortfolioDetail.tsx:238-292` already prints, minus its Actions column. `normalizeTaxSummary`
    keeps only `currency` today, so this slice adds two fields to the hand-written type and two
    lines to the normalizer. Both new fields are **optional** (`strategy?: string; threshold_days?:
    number`) so the existing `TaxSummary` literals in `PortfolioDetail.test.tsx` and
    `Dashboard.test.tsx` still compile under `npm run typecheck`; the money fields stay `number`.
*   **Which currency formats the money, and which prints in the header.** Money formats with the
    payload's `currency`; the header prints the **portfolio object's** base currency. Not two
    competing sources: the populated branch returns `currency` **as** `portfolio.base_currency`, so
    they are the same string whenever there are holdings to format. They can differ only on the empty
    branch, which returns no `currency` key at all and would otherwise default to `USD` on a EUR
    portfolio — and that branch prints no money (Goal 7).

### What "splits" and the strategy line mean

Two terms on the printed page are specific to this project and are not general engineering
vocabulary.

*   **Splits.** Stock splits, auto-detected and persisted by the tax-summary handler itself —
    `get_portfolio_tax_summary` calls `sync_splits_for_asset` per asset (`analytics.rs:63-65`)
    before it computes anything. So the caveat means: the split rows in the database when the request
    ran, some of which the handler may have just added from the provider. It matters because a split
    can be recorded long after it happened, which silently rewrites history; the hazard is tracked as
    issue #50. The printed string is the `splits-note` row in *On-page copy*.
*   **`HYBRID` and `threshold_days`.** The engine supports `FIFO`, `LIFO` and `HYBRID`; the strategy
    match lives in `replay` (`tax_engine.rs`, the `match strategy` block). `HYBRID` consumes lots
    held for **`threshold_days` or fewer** first — the short-term bucket — and `threshold_days` is
    that short/long boundary in days. It is meaningless for `FIFO` and `LIFO`: `term_for`
    (`tax_engine.rs:645-649`) returns `None` for anything but `HYBRID`, which is why the run-context
    line appends it only for `HYBRID`. The on-screen precedent is `SellPnlModal.tsx:210`.
*   **What the strategy line can claim.** This route defaults but does not **normalize** strategy
    (`analytics.rs:59`), so an unrecognized value would print verbatim while the engine silently used
    FIFO. The app cannot send one: strategy state is initialised to `'FIFO'` (`App.tsx:39`) and set
    only from the three-option select (`PortfolioDetail.tsx:179-181`). So the claim is "the strategy
    the app sent", not "the strategy the engine used". Normalizing this route is backend work and is
    recorded in the parent design's Target File 6, which already edits this handler.

### Palette inversion

The palette is variable-driven (`index.css:3-31`), and components read it through the variables —
including inline styles such as `style={{ color: 'var(--text-secondary)' }}`
(`SellPnlModal.tsx:206`). That is what makes this low-effort: `print.css` overrides the variables on
`:root` inside `@media print` and most of the app flips at once, inline styles included.

`frontend/src/print.css` (new, flat beside `index.css`) is imported from `main.tsx` after
`index.css`, so it wins the cascade at equal specificity. That is source order doing the work, not
the file boundary: a `@media print` block appended to `index.css` would win just as reliably. The
separate file is for organisation; its cost is that the print palette is not next to the palette it
inverts.

**The hide rules are scoped to the statement tab by `:has()`.** Unscoped rules are not acceptable:
rule 2 as `.main-content > *:not([data-print="statement"])` hides every direct child of
`.main-content` on the dashboard, analytics and ledger tabs, because nothing there carries that
attribute — Ctrl-P on three of four tabs would print a blank A4 sheet, where today it prints the
dashboard. `:has()` expresses the scoping with no JavaScript, so there is no lifecycle to get wrong
and no attribute that can outlive the component; the repo touches `document.body` exactly once
today, to append a download anchor (`utils/download.ts:33`), and this slice does not change that.
`:has()` is baseline (Chrome 105, Safari 15.4, Firefox 121) and the repo sets no browserslist
override.

It does five things, in this order:

1.  `@page { size: A4; margin: 14mm }`. Letter users change it in the print dialog. `@page` cannot
    be scoped by a selector, so this applies to a print started from any tab; the consequence is
    accepted under *Other tabs*.
2.  **Hide the shell, not a list of things inside it.** Two rules, in this order:

    ```css
    .app-container:has([data-print="statement"]) > *:not(.main-content) { display: none }
    .main-content:has([data-print="statement"]) > *:not([data-print="statement"]) { display: none }
    ```

    The first removes the sidebar (`<aside className="sidebar">`, `App.tsx:379`) and every other
    direct child of `.app-container` — which includes the auth-failed alert (`App.tsx:359-377`, the
    `auth_error=auth_failed` banner), a `position: fixed` full-width red banner that an enumerated
    hide list would print on every page. The second leaves only the statement in the content column,
    so the header row (`<div className="header-row">`, `App.tsx:458`) and any chrome added there
    later disappear without anyone maintaining a list. An allowlist of what to hide drifts every time
    someone adds chrome, in the same direction, silently; the inclusive form cannot miss a direct
    child.

    **Two structural preconditions, because both rules use the child combinator `>`.** The
    statement's root element must carry `data-print="statement"` **and be a direct child of
    `.main-content`** — not wrapped in a layout `<div>` or `<section>`. Wrap it and rule 2 hides the
    wrapper: a blank printed page from a tab that looks correct on screen. The Print control sits in
    the same position, as a second direct child of `.main-content` beside the statement root, which
    is what hides it. The class names `.app-container` and `.main-content` are load-bearing for
    printing; renaming either in a later refactor silently breaks the print path. The smoke's
    visibility assertion is what catches this, and only when someone runs it.
3.  Collapse the layout: `.app-container` is a `260px 1fr` grid (`index.css:93-98`); in print it
    becomes one column, or the page prints with a 260 px gutter of nothing.
4.  Invert the palette. The six values are fixed here rather than left to implementation, because
    layer 2's colour assertion compares against them literally:

    ```css
    --bg-dark: #ffffff;   --bg-card: #ffffff;   --border-color: #333333;
    --text-primary: #111111; --text-secondary: #333333; --text-muted: #555555;
    ```

    plus `.statement-table td { border-bottom: 1px solid #333333 }`, because `.data-table td`'s own
    border is a literal `rgba(255,255,255,0.03)` (`index.css:247`) and not a variable — without the
    override the header rule inverts and the body rules do not, giving a half-inverted table.
    `.glass-card` loses `backdrop-filter`, `box-shadow` and its translucent background. **Which of these carries Goal 6:** legibility comes entirely from the
    **text** overrides, because text is always painted and does not depend on the background-graphics
    setting. The background and border overrides are redundant in the Goal 6 sense and load-bearing
    in the other one: a reader who *has* enabled background graphics gets dark text on the dark app
    background without them. The two groups are not interchangeable. In the statement, sign is
    carried by the number, not by colour — `formatCurrency` renders negatives as `-$1,234.56`, so a
    greyscale printer loses nothing. (Elsewhere in the app it is coloured:
    `PortfolioDetail.tsx:286-292`.)
5.  Break rules: `tr { break-inside: avoid }`, and the headline block never splits.

No opt-out rule (`[data-no-print]`) is specified. Nothing inside the printed subtree needs one today:
the only known opt-out case is the Print control, and the sibling-position rule already hides it.
Add the rule in the change that first needs it.

#### Other tabs, and what will not invert

With the hide rules scoped as above, printing from the dashboard, analytics or ledger tab keeps
its layout: sidebar, navigation, charts and all. That is not designed, it is unchanged, and it
is out of scope — the statement is the tab you print from, and the only one the smoke exercises.
What changes for those tabs is **not** only `@page`, which no selector can scope: they now get A4
with 14 mm margins, and the palette overrides at `print.css:36-49` sit inside `@media print` with
no selector of their own, so their text and card backgrounds invert too. What does not invert on
those tabs is anything with a hardcoded color, and the sidebar is the obvious case — `.sidebar`
keeps `rgba(10, 15, 28, 0.95)` (`index.css:101`) while its nav text becomes `--text-secondary`
`#333333`, so with background graphics enabled that sidebar prints dark-on-dark. The statement
tab is unaffected, because rule 1 hides the sidebar there.

Two backgrounds are not variables and do not invert: `.badge-crypto`'s background
(`index.css:334`; its **color** is a variable and does invert) and `.data-table tr:hover td`
(`:252`, a hover state that cannot occur on paper). `.sidebar` is hardcoded at `:101` as well —
rule 1 hides it on the statement tab, so it is not a residual *there*, and it is one on the other
four tabs, as just said.

### Verification layers, and what each cannot cover

1.  **Vitest component test** (`frontend/src/test/components/PortfolioStatement.test.tsx`). jsdom
    does not apply print CSS, so this proves structure and formatting only.

    The component takes `taxSummary`, `portfolio` and `fetchedAt` as **props** (see *The component's
    interface*) and the test uses a **prop fixture**, which is how both existing tax-summary consumers
    are already tested (`PortfolioDetail.test.tsx:27-55`, `Dashboard.test.tsx:22-42`). The shared MSW
    handler at `test/mocks/handlers.ts:121` is therefore **not** touched; if an App-level test is ever
    wanted, the repo's convention for per-test payload variation is a runtime override
    (`AppLedger.test.tsx:18-20`, `SellPnlModal.test.tsx:116`), not branching the shared handler.

    *Formatting* is asserted by a **golden**, with the clock frozen as specified in *On-page copy*. The
    fixture is `currency: 'EUR'`, `total_realized_pnl: 1234.565`, and the test asserts
    `getByTestId('figure-realized').textContent === '€1,234.57'` as a literal — verified against
    `formatters.ts`, which routes EUR through `Intl` at 2 dp and the nearest double to `1234.565`
    sits above the midpoint. Anything of the form "the printed figure equals the payload value rendered
    through `formatters.ts`" cannot fail for a component that calls `formatCurrency`, and deriving the
    expected value by rounding the payload with a double re-runs the renderer's own path. No assertion
    here may compare two normalizers.

    *Structure, populated fixture*: the statement root carries `data-print="statement"`; the printed
    subtree contains no `button`, `input`, `select` or `svg`; `figure-value`, `figure-realized` and
    `figure-unrealized` are each present and non-empty; every hook in *On-page copy* except
    `empty-note` is present with its specified text; `holdings` has the nine `<th>` strings from
    *Structural hooks and the holdings table* and one `<tr>` per asset; the run-context line carries
    the payload's echoed `strategy` and `threshold_days`, with `threshold_days` absent for a
    non-HYBRID fixture.

    *Structure, empty fixture* (`assets: []`): `empty-note` and `generated-at` present, and **absent**
    — `data-at`, `rounding-note`, `splits-note`, `run-context`, and all three figure hooks.
2.  **Playwright smoke** (`frontend/e2e/statement.smoke.mjs`, following `sell-pnl.smoke.mjs`).
    Navigate with `page.getByText('Printable Statement').click()` (Alternative G was rejected, so there
    is no URL to go to). Then `page.emulateMedia({ media: 'print' })` and assert:

    *   the statement is **visible** — non-zero box, and computed `color` on `[data-print="statement"]`
        and on `[data-testid="figure-realized"]` is `rgb(17, 17, 17)`, the `--text-primary` value from
        *Palette inversion* step 4. **Text color only.** Do not assert a light background: Goal 6's
        premise is that background graphics are off, `emulateMedia` does not enable background
        painting, and `page.pdf()` defaults `printBackground: false`.
    *   nothing printable is inside the printed subtree:
        `expect(await page.locator('[data-print="statement"] button, [data-print="statement"] input, [data-print="statement"] select, [data-print="statement"] svg').count()).toBe(0)`.
        This is the automated form of "no Print control", and layer 1 cannot cover it because it
        renders the component in isolation and never sees the control.
    *   `.sidebar` computes `display: none`. The auth alert is **not** asserted: it renders only from
        the `auth_error=auth_failed` query param and is not in the DOM under this harness.
    *   no horizontal overflow, measured **on the statement**, not the document:
        `expect(await el.scrollWidth()) <= (await el.clientWidth())` for `[data-print="statement"]`.
        A document-level check is meaningless here — `index.css:47` sets `body { overflow-x: hidden }`,
        which propagates to the viewport, so the document never reports overflow; it clips silently.
        Note `emulateMedia` does not resize the viewport (the harness runs 1400×1000), so this checks
        the print stylesheet's typography at desktop width; the A4 case is the manual check's.
    *   `page.pdf({ format: 'A4', preferCSSPageSize: true, printBackground: false })` is non-empty.
        The options are stated because Playwright's defaults are Letter with
        `preferCSSPageSize: false`, which would measure a page size this slice does not ship. **No
        page-count assertion**: proving "one page" needs a PDF parser, and no such dependency exists in
        `frontend/package.json`. Decided item 2's one-page claim moves to the manual check.
    *   **no figure is asserted.** Figures come from a live price provider; when it is unreachable the
        fallback returns zero, and a smoke that asserted figures would then print zeros and still
        pass. The smoke pins structure and visibility, which are unaffected by price. The seeded
        holdings are denominated in the portfolio's base currency — a foreign-currency fixture would
        500 the request before any print assertion ran (see *When there is no payload at all*).

    **Local step, not a gate.** `e2e/run.sh` hardcodes `node e2e/sell-pnl.smoke.mjs` and gains a
    target argument here; `ci.yml` has no browser job, which is issue #45. The full reasoning —
    including why a CI browser job is possible but not free — is in `decision_records.md` §3 and is
    destined for #45 and `frontend/e2e/README.md`.

    **Auth prerequisite, fixed in this slice.** The smoke drives a real browser at a real backend, so
    the run needs `AUTH_MODE=disabled`, which `run.sh` does not set — and without it the server refuses
    to start, so the existing harness does not run at all. The one-line export lands here beside
    `run.sh`'s existing `DATABASE_URL` export, following `start.sh:21`. Seeding is through the REST
    API as `sell-pnl.smoke.mjs:36-59` already does, needing no browser login; the fixture is two
    holdings in one portfolio with the strategy passed explicitly, so the printed run context is
    known rather than defaulted. That seeding block is copied rather than shared: two callers do not
    make a helper, and #45 can extract one if it inherits a third.
3.  **A human with a print preview.** `docs/auth-testing.md` §5 is the manual browser walkthrough for
    what CI cannot check; the statement's print preview joins that list. Check: no sidebar, no nav, no
    Print control, dark text on white, one page, and the headline figures equal to the payload shown
    in devtools. Not "match the dashboard" — `Dashboard.tsx:39-41` falls back to `metrics.*` when a
    tax-summary figure is falsy, so a coincidental fallback can satisfy an eyeball comparison.

### Sequencing

One slice, sequenced first among the three programme slices. There is no useful smaller cut: the page
without the print stylesheet is a dashboard with a different heading, and the print stylesheet
without the page has nothing to style. It reaches no network in tests other than the local smoke, and
moves no existing figure. The programme-level slice table is owned by the parent design
(`2026-11-09-tax-report-and-statement-design.md`, *Sequencing*); this document does not restate it.

## Target Files

### 1. `frontend/src/components/PortfolioStatement.tsx` (Create)

*   **Type / Scope:** the printable view.
*   **Type / Scope:** the printable view.
*   **Proposed Changes:** renders the headline block, the provenance block, and one row per holding
    (Decided item 1). Props, hooks, table columns and per-column renderers are all specified in *The
    component's interface* and *Structural hooks and the holdings table*; this file list does not
    restate them. No asset-subset filter — the only arithmetic that would make a filtered headline
    honest is the arithmetic renderers are forbidden from doing. Derives no totals. No `button`,
    `input`, `select` or hardcoded color inside the printed subtree.

### 2. `frontend/src/print.css` (Create) and `frontend/src/main.tsx` (Modify)

*   **Type / Scope:** the print stylesheet, imported after `index.css` so it wins the cascade at
    equal specificity.
*   **Proposed Changes:** the five rules in *Palette inversion*, with the `:has()` scoping and the
    two structural preconditions named exactly as written there.

### 3. `frontend/src/utils/decimal.ts` (Modify) and `frontend/src/types.ts` (Modify)

*   **Type / Scope:** surface the two run-context fields the payload already sends.
*   **Proposed Changes:** `normalizeTaxSummary` keeps `strategy` and `threshold_days`
    (`decimal.ts:63` keeps `currency` only today) and `TaxSummary` gains both as **optional**
    fields, so the existing literals in `PortfolioDetail.test.tsx` and `Dashboard.test.tsx` still
    compile under `npm run typecheck`. Additive; no existing figure moves. There is no second
    normalizer and no string-typed mirror type — Alternative C1.

### 4. `frontend/src/App.tsx` (Modify)

*   **Type / Scope:** the fifth nav item and the tab branch.
*   **Proposed Changes:** renders the statement from the `taxSummary` state `fetchPortfolioData`
    already sets, guarded the way the sibling tab guards (`activeTab === 'portfolios' && taxSummary`,
    `App.tsx:483`).
*   **Stale payload.** `setTaxSummary` runs once (`App.tsx:114`) and is never cleared, so a portfolio
    switch leaves the previous portfolio's summary in state until the new fetch resolves — and a page
    with B's numbers under A's heading is the one failure this artifact cannot absorb quietly, because
    unlike the zeros case it looks right. The payload carries no portfolio id, so record the id beside
    the summary and guard only the statement branch. The shape is `taxMeta: { portfolioId, fetchedAt }`
    set beside `setTaxSummary`, specified in *The component's interface*. The repo has the pattern and
    its rationale: `App.tsx:68` stores `{ txId, portfolioId }` with the guard at `:518`, and the comment
    at `:66-67` explains why it is derived rather than cleared from an effect — *"five places change
    `selectedId`, and a reset that has to be remembered in each one is a reset that gets forgotten."*
    A parallel record, then, not a reshape of state three tabs consume; and no clear-on-switch variant,
    which that comment already rejects.
*   **Currency:** see *Money*, "Which currency formats the money, and which prints in the header".
*   **Fetch timestamp:** `fetchPortfolioData` records the time beside `setTaxSummary` so the `data-at`
    line has a source.

### 5. `frontend/src/test/components/PortfolioStatement.test.tsx` (Create)

*   **Type / Scope:** the component test.
*   **Proposed Changes:** the assertions listed under *Verification layers* layer 1, with a prop
    fixture. No change to `test/mocks/handlers.ts`.

### 6. `frontend/e2e/statement.smoke.mjs` (Create), `frontend/e2e/run.sh` (Modify), `frontend/package.json` (Modify)

*   **Type / Scope:** the browser smoke, and the parameterisation it needs.
*   **Proposed Changes:** as in *Verification layers* layer 2. `run.sh` gains a target argument
    defaulting to the existing smoke, and gains `export AUTH_MODE=disabled` beside its existing
    `DATABASE_URL` export. `package.json` gains `e2e:statement`. **Not wired into CI** — issue #45.

### 7. Not built — no print-CSS grep gate

Carried from the parent design. A grep asserting a stylesheet mentions some selectors goes green while
the page still prints badly. The Playwright smoke is the check; see Alternative B.

## Risks

| Risk | Evidence | Mitigation |
| :-- | :-- | :-- |
| **The print path ships unverified.** No CI browser job; the smoke is a local step. | `ci.yml` has no browser job; issue #45 open | Selectors and smoke assertions are named here and handed to #45. The manual print preview is listed in `docs/auth-testing.md` §5. |
| **The e2e harness does not currently start**, so the smoke's acceptance step cannot run. | `run.sh` sets no `AUTH_MODE`; the server refuses to start without OIDC config | This slice adds `export AUTH_MODE=disabled` to `run.sh` (Target Files 6), following `start.sh:21`. |
| **Palette override misses a hardcoded color inside the statement.** | `SellPnlModal.tsx:131` uses inline `background: '#121929'`; `PortfolioDetail.tsx:286-292` colours P&L with `var(--color-success)`, which step 4 does **not** override; `.data-table td`'s border is a literal `rgba(255,255,255,0.03)` (`index.css:247`) | The statement avoids all three: no hardcoded colors, no success/danger colouring (sign is in the number), asset type as plain text rather than `.badge`, and an explicit print override for `.statement-table td`'s border. The smoke's text-color check catches a hardcoded **text** color and **cannot** see a hardcoded background; that residual is accepted rather than gated, for the reason in `decision_records.md` §5. |
| **A printed figure differs from the CSV's.** | The CSV will be full-precision (`docs/superpowers/reviews/2026-11-05-sell-pnl-breakdown.md:18`); `normalizeTaxSummary` converts money to `number` before any statement code runs (`decimal.ts:64-66`) | Goal 3 declares the rounding on the page, so the difference is stated rather than discovered by an adviser. Verification layer 1 pins the reachable third-decimal case. |
| **A third renderer of one payload, with nothing comparing it to the second.** | `PortfolioDetail.tsx:238-292` renders the same `taxSummary.assets`; `Dashboard.tsx:40-45` renders the same headline figures | Accepted knowingly (Alternative D). The statement is a leaf with no interactivity. If the two tables ever need to agree, the check is a comparison test between them, and it is not written here. |
| **A4 on Letter hardware.** | `@page { size: A4 }`, and `@page` cannot be scoped, so other tabs' paper size changes too | Stated in the dialog, not hidden. The owner's portfolios are EUR-denominated, which is a hint about likely paper preference and nothing more. |
| **Statement read as a quarterly figure.** | The undated endpoint has no window | Goal 1's label requirement: "cumulative since inception" is on the page, not implied. |
| **Empty portfolio prints zeros.** | `analytics.rs:45-51` returns zeros with no `currency` | Goal 7 and *Empty portfolio*: heading, printed date and the empty note only. The negative list is asserted. |
| **A stale payload prints under the wrong portfolio name.** | `setTaxSummary` is called once (`App.tsx:114`) and never cleared | Target Files 4 gives the form: record the id beside the summary, guard the statement branch, following `App.tsx:68`. |
| **A print started from another tab is not designed.** | The hide rules are scoped to the statement tab, so other tabs keep their layout | Out of scope and stated under *Other tabs*. Those tabs get two changes they did not ask for: `@page` (A4 with 14 mm margins), which no selector can scope, and the unscoped palette overrides (`print.css:36-49`), which invert their text and cards. The sidebar's hardcoded background does not invert along with them. |
| **Foreign-currency figures are wrong, inherited.** | The pricing-layer spec records that foreign-currency assets cannot be priced correctly today | Not this slice's fix and not this slice's regression. The statement inherits the dashboard's existing behaviour; the pricing-layer change corrects both at once. |
| **A failed tax-summary request prints the app shell, not a statement.** | `taxSummary` stays `null` → the guard renders nothing → the `:has()`-scoped hide rules do not match. The FX cause recorded at drafting (`currency_service.rs:234` → 500) was closed by the pricing-layer change | **Accepted gap**, specified in *When there is no payload at all*. The print gets the shell — sidebar, header, error banner — around an empty content column, in the inverted palette, because the palette overrides carry no selector. Not the dashboard, and not illegible. |

## Decided during drafting

Two items were open at first draft. Each is settled here, with what it trades away. A third — the
echoed strategy — is recorded in `decision_records.md` §4.

1.  **The statement lists holdings.** The headline figures are the point, and a 40-row table is a
    second page and a second set of numbers to reconcile against the CSV. It lists them anyway: a
    statement without holdings is a receipt, and the owner's use case is handing something to a
    person who will ask "which positions?". The reconciliation cost is bounded by Goal 3 — every
    printed row is a payload value rounded once at display, so a row cannot disagree with the CSV
    except by the rounding the page declares. What the rows can do is run to a second page, which is
    a layout fact, not a correctness one.
2.  **Page count is a manual check, not an assertion.** The original form of this decision was "the
    smoke asserts one page for its own fixture and says nothing about larger portfolios" — but Stage 9
    found that counting pages needs a PDF parser, and no such dependency exists in
    `frontend/package.json`. A bound that holds for 50 holdings is a layout claim nobody has measured
    anyway, and an assertion that fails on the owner's real portfolio but passes locally is an
    assertion that teaches people to ignore the smoke. The page-break rules still ship
    (`tr { break-inside: avoid }`); the page count is confirmed by eye in the print preview.

## Alternatives Considered

### Alternative A: server-generated PDF

*   **Pros**: exact pagination, no browser print dialog, a real artifact to hash and file.
*   **Cons**: a new Rust dependency with a font-file requirement (`printpdf`, `genpdf`) or a headless
    browser on the server. The app runs in a single-replica pod with a 256 Mi memory limit
    (`k8s/app-deployment.yaml:78`, `replicas: 1` at `:9`) — PDF generation or a headless browser is
    the kind of work that turns a working pod into an OOMKill.
*   **Reason for rejection**: the statement is read or emailed, not filed or signed. The CSV is the
    filed artifact, and when the tax report ships its CSV is specified to be exact. Browser
    print-to-PDF reuses the existing formatters, so the numbers look like the rest of the app for
    free.

### Alternative B: a print-CSS grep gate in CI

*   **Pros**: something runs on every push; cheap.
*   **Cons**: it asserts that a stylesheet mentions selectors, not that the page prints. It passes on
    a page that prints white-on-white.
*   **Reason for rejection**: a gate that can pass while the feature is broken converts "unverified"
    into "verified and therefore fine". The Playwright smoke is the check; it is a local step until
    issue #45 lands,
    the same blocker as the existing sell-P&L smoke.
*   **Scope note**: what is rejected is a grep over a *stylesheet*. A grep over
    `PortfolioStatement.tsx` for a hardcoded color literal is a different check and is not rejected
    by this argument; it is out of this slice for the reason in `decision_records.md` §5.

### Alternative C: make the printed page exact

Two sub-options, both rejected.

*   **C1 — preserve the payload's decimal strings through to the page.** Rejected: the conversion
    happens in `normalizeTaxSummary`, the shared normalizer every screen uses, and `formatCurrency`
    coerces its argument to a number whatever it is given, so `formatCurrency("1234.565")` and
    `formatCurrency(1234.565)` are byte-identical. The change would add a second payload shape and a
    drift duty while altering nothing observable. Derivation in `decision_records.md` §1.
*   **C2 — round with a decimal library so the page is exact.** Rejected: exactness is the filed
    artifact's job, and a page printing `1234.56500000000000000000` to a person is worse than a page
    that says it rounded. It also means a second rounding implementation to keep in step with the
    engine's, where the repo's only precedent for decimal shaping in the frontend is the partial one
    at `sellPnlCsv.ts:61-66` — which handles the exponent case that a naive `toFixed` gets wrong at
    `1e21`, and is exactly the kind of detail a second rounder would have to re-derive.

### Alternative D: a print mode over the view that already renders this payload

*   **Pros**: no third renderer. `PortfolioDetail.tsx:238-292` already renders a holdings table from
    `taxSummary.assets` — symbol, type, shares, average cost, price, market value, realized P&L,
    unrealized P&L, ROI — which is Decided item 1's row set plus an Actions column, and
    `Dashboard.tsx:40-45` already renders the headline figures from the same payload.
*   **Cons**: the existing table is exactly what Goal 2 forbids. It has buttons, expanders, an Actions
    column, and inline `var(--color-success)` colors on the P&L cells (`PortfolioDetail.tsx:286-292`);
    it takes its currency from `portfolio?.currency || 'USD'` (`:261`) rather than from the payload.
    Making it print well means suppressing most of it and re-deriving what the leaf component gets
    for free.
*   **Reason for rejection**: a dedicated leaf is the cheaper artifact. The cost is recorded plainly:
    **this slice adds a third renderer of one payload.** The two tables will disagree about columns,
    currency source, and sign and rounding conventions unless something compares them, and neither the
    smoke nor the golden does.

### Alternative E: the same route serves server-rendered HTML

*   **Pros**: a real document with no browser print dialog, no CSS cascade to fight, and pagination the
    server controls.
*   **Cons**: blocked on the pricing layer, because a server-rendered statement would compute its own
    figures and inherit the same FX defects with no dashboard beside it to make them visible. It also
    duplicates the frontend's `Intl` formatting rules in Rust, which is a second formatting
    implementation with no golden to hold them together.
*   **Reason for rejection**: the frontend already has the payload, the formatters, and the palette
    variables that make inversion cheap. Server rendering trades a CSS problem for a formatting-
    parity problem, which is the harder one.

### Alternative F: a copy-to-clipboard text block instead of a printed page

*   **Pros**: no print stylesheet at all, no palette inversion, works on any device, and the text can
    be pasted into an email to the adviser.
*   **Cons**: it is not a document. It has no page, no header, no provenance block that reads as
    belonging to the figures, and no way to attach it to a tax filing.
*   **Reason for rejection**: Decided item 1 is an explicit owner decision that the artifact is a
    document. The local-step decision recorded under *Verification layers* layer 2 stands on its own
    and does not depend on this one.

### Alternative G: `?view=statement` as a poor man's deep link

*   **Pros**: `App.tsx:58` already reads `URLSearchParams` and `:347-353` already rewrites the query
    string, so a view parameter is a few lines and buys a bookmarkable URL and a smoke that can
    `goto` the statement instead of clicking a nav item.
*   **Cons**: `selectedId` is not in the URL either, so the parameter names a tab without naming the
    portfolio it is about. Half a deep link is not a deep link: a bookmarked `?view=statement` opens
    whatever portfolio the session last selected.
*   **Reason for rejection**: it buys a partial version of a thing this slice does not promise, and
    the smoke can click a nav item. If the app ever grows a router, the route replaces this rather
    than extending it.

## Verification commands

*   `npm test`, `npm run lint`, `npm run typecheck` from `frontend/`.
*   `npm run e2e:statement` from `frontend/` — **local step, not a gate**. Needs a chromium binary and
    fonts; on NixOS,
    `nix-shell -p chromium -p liberation_ttf --run './e2e/run.sh statement'`. It also needs the
    `AUTH_MODE=disabled` export described in *Verification layers* layer 2, which lands in `run.sh`
    with this slice.
*   Manual: open the statement, print preview, confirm no sidebar, no nav, no Print control, dark text
    on white, one page, and that the headline figures equal the payload shown in devtools — not
    "match the dashboard", for the fallback reason in *Verification layers* layer 3. Added to the
    manual browser walkthrough in `docs/auth-testing.md` §5.
*   Cross-check **once the tax report ships** — it does not exist yet, and its date semantics are still
    open in the parent design: same portfolio, same moment, the statement's cumulative realized
    equals the CSV's cumulative realized column rounded the way `formatters.ts` rounds it, and the
    page carries the line saying it was rounded. Until then the shipped equality is the golden-value
    test in layer 1, evaluated against the payload.
