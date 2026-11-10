# Portfolio Statement (Printable)

*   **Author:** javierarrieta, with EGM review support
*   **Status:** Proposed
*   **Date:** 2026-11-09
*   **Depends on:** `2026-11-09-pricing-layer-truthfulness.md` (dated FX — a statement that
    converts at the wrong date is worse than no statement). **Related:**
    `2026-11-09-tax-report-and-statement-design.md` (the tax report; this document was
    split out of it).
*   **Origin:** split out of `2026-11-09-tax-report-and-statement-design.md` during that
    document's review; the scope judgement that split it is stage 8 of
    `docs/superpowers/reviews/2026-11-09-tax-report/zoom_out_2_report.md`.
*   **Naming note:** below, **"the parent design" always means the tax-report design named
    above**, not the pricing-layer document listed under *Depends on*. Where a claim rests on a
    quotation from it, the file is named again.

## Overview

A printable page for one portfolio: realized P&L cumulative from inception, unrealized P&L
as of today, and the holdings behind them. The owner reads it or hands it to an adviser. The
browser prints it to PDF; the server never produces a PDF.

It takes no date. The tax report is the dated artifact; a statement about a past quarter is
not on the roadmap this document covers. Because it is always today, **it needs no backend
change at all**: it renders the `tax-summary` payload the app already fetches.

That is the whole appeal of this slice and also its limit. Everything hard here is about
printing a dark-themed app honestly, and about a page whose figures are rounded saying so
instead of pretending to be the exact file next door.

## The Problem

*   **There is no printable view.** Verified: no `@media print` rule exists anywhere in
    `frontend/src/`. The owner's only option today is Ctrl-P on the dashboard, which prints
    the sidebar, the navigation, the error banner, and a Recharts SVG.
*   **The app is dark glassmorphism, and dark themes do not print.** `index.css:6-30` sets
    `--bg-dark: #090d16` and `--text-primary: #f8fafc`, and `.glass-card` uses
    `backdrop-filter: blur(16px)`. Browsers do not print background colors unless the user
    turns on background graphics, so a naive print is near-white text on white paper —
    invisible, not merely ugly. The print stylesheet's real job is **palette inversion**,
    not hiding the nav. Hiding the nav is the easy third.
*   **The printed page must say what it rounded.** The backend sends money as full-precision
    decimal strings (`db_types.rs:31`) and the engine multiplies by the FX rate with no
    `set_scale` anywhere (`tax_engine.rs:418`, `:422`, `:433`), so a realized figure carries
    more decimals than a page should show. The frontend quantises at display: `Intl` renders
    2 dp for fiat, up to 8 for `BTC`/`ETH` (`formatters.ts:10-11`). A printed figure and the
    tax report CSV's figure for the same holding are therefore **guaranteed to differ** — not
    by a floating-point accident but by design, one being quantised and the other not. A page
    printing `1,234.57` beside a CSV cell reading `1234.56500000000000000000`, while claiming
    to be the same number, is the defect. The fix is to say the rounding out loud on the page.
*   **There is no router, so the statement is not linkable.** `App.tsx:36` is
    `useState(DEFAULT_TAB)`; the nav is four `<li className="nav-item">` elements
    (`App.tsx:385-396`). A printed page cannot be re-opened by URL or bookmarked, and this
    document does not intend to solve that. A query parameter is rejected under Alternative G.
*   **Nothing can prove it prints.** `.github/workflows/ci.yml` has no browser job and issue
    #45 (*Run the sell P&L Playwright smoke in CI*) is open, so this slice **ships unverified
    by design**. The reasons it stays that way are in Verification layer 2, along with the
    correction that CI *could* run it — this is a choice, not an impossibility. Selectors and
    smoke assertions are named here rather than left to implementation time, so the check is
    specific the moment #45 lands.
*   **The person the statement is for cannot log in.** Every `/api` route takes `AuthedUser`
    plus a `require*` call, and `apiFetch.ts:22-26` sends an unauthenticated caller to
    `/auth/login`. An adviser has no account. The artifact necessarily leaves the app: the
    owner prints, then forwards. So the page must stand on its own once detached, which is what
    Goal 4's provenance block and Goal 3's declared rounding are for. A copy-to-clipboard text
    block is considered and rejected under Alternative F.

## Goals

*   **Goal 1 — the P&L figures, honestly labelled.** Cumulative realized P&L and unrealized
    P&L for the selected portfolio, with realized labelled as **cumulative since
    inception**; portfolio value prints alongside them and needs no such label. The undated
    `tax-summary` endpoint has no window, so a statement cannot show a quarterly realized
    figure; an unlabelled number next to a quarterly tax form is how a wrong filing starts.
*   **Goal 2 — the printed page is a document.** No sidebar, no navigation, no buttons, no
    inputs, no charts, no error banner. One column. `@page { size: A4; margin: 14mm }`.
*   **Goal 3 — the printed numbers are the payload's numbers, quantised once and declared.**
    The statement is a **display artifact**. Every money value is a payload value rendered
    through `frontend/src/utils/formatters.ts` and quantised at display, nowhere else — to
    whatever `Intl` decides for that currency code: 2 dp for `EUR`/`USD`, **0 dp for
    zero-decimal codes** (`JPY` renders `¥1,235` from `1234.565`), up to 8 dp for `BTC`/`ETH`
    (`formatters.ts:10-11`). That is the rule every other screen already follows, and it is
    stated as "whatever `formatters.ts` renders" rather than "to the cent" because the cent
    phrasing is false for crypto *and* for zero-decimal fiat. The statement performs no
    arithmetic: it sums nothing, differences nothing, never re-derives a total. The payload
    carries full precision (`db_types.rs:31`), so the page prints a visible line saying its
    figures are rounded from that precision. When the tax report ships, its cumulative realized
    column should equal the statement's figure rounded the same way `formatters.ts` rounds it;
    that cross-check is conditional on the tax report existing, and the test that ships with
    this slice is the golden under Verification, which needs no second rounder to evaluate.
*   **Goal 4 — the page states its own provenance.** Portfolio name, base currency, the
    strategy and `threshold_days` **as the server echoed them** (`analytics.rs:98-100`), the
    generation date, and the splits caveat. `threshold_days` prints only for `HYBRID`, because
    it is unused by `FIFO` and `LIFO` (`tax_engine.rs:645-649`) and printing it next to `FIFO`
    would present an unused parameter as part of the method; see *What "splits" and the strategy
    line mean* for both terms. The empty-portfolio branch at `analytics.rs:45-51` echoes
    **none** of the three fields, so the run-context line is omitted there rather than rendered
    from `str()`/`num()` defaults — a line reading "realized · (threshold 0d)" would be a
    fabricated run context on the one branch already cited for the currency problem.
*   **Goal 5 — no backend change in this slice.** No new route, no new field, no new figure on
    any existing endpoint, no OpenAPI regeneration. This is a **sequencing** boundary, not a
    standing principle: the parent design already rewrites this handler's response — which is
    why the committed OpenAPI schema shows no body for this response (the handler returns
    `Json<serde_json::Value>`, so `--print-openapi` has nothing to describe) — and will add
    `from`/`to` to it. If this slice
    needs the backend, it has drifted out of scope and should stop. The boundary's cost is
    stated: the run context printed here is a subset of what the parent design will carry
    anyway. It is kept because any response-shape change lands in `docs/openapi/openapi.json`,
    which CI diffs byte-for-byte, so it is not a quiet two-field change — and because the
    statement is the one slice of this programme not blocked on the pricing layer.
*   **Goal 6 — the palette inversion does not depend on a browser setting.** It must not
    rely on "background graphics" being ticked. Text becomes dark because the stylesheet
    says so, not because the browser kept a background.
*   **Goal 7 — the empty portfolio says so.** `analytics.rs:45-51` returns three zeros and no
    holdings. Printing three zeros under a "Portfolio Statement" heading is a lie by omission,
    so that branch prints one line instead — *`No holdings in this portfolio on {date}.`* —
    with no headline figures, no run-context line and no holdings table. See Empty portfolio
    below for where that lives in the code.

## Non-Goals

*   **No date parameter, and no past-period statement.** Decided in the parent design.
*   **No server-side PDF, and no PDF dependency.** See Alternative A.
*   **No print-CSS gate in CI.** See Alternative B and Risks.
*   **No router, no deep link, no shareable statement URL.**
*   **No per-asset price-date provenance on the statement.** The undated endpoint returns no
    `price_date_used`; the CSV is the provenance artifact. The statement says what it knows
    and points at the CSV for the rest.
*   **No cross-portfolio statement.** One portfolio, as in the tax report.
*   **No print theme picker, no "dark print" mode.**

## Technical Plan

### Surface: a fifth tab, not a route

`statement` joins `dashboard`, `portfolios`, `analytics` and `ledger` in the existing nav
(`App.tsx:385-396`). It renders from the `taxSummary` state that `fetchPortfolioData`
already populates (`App.tsx:111`), so opening the statement costs no request and cannot fail
in a way the dashboard does not already fail.

The Print control is a button **outside** the printed subtree, marked `data-no-print`,
calling `window.print()`. A statement whose print button prints itself with a print button
on it is a statement that ships with a screenshot of its own UI.

### On-page copy, written out

Three strings are requirements, not implementation detail — Goals 1, 3 and 7 each assert one —
so they are written here with a stable hook each, for the same reason the selectors are: an
assertion that says "the rounding line is present" is not an assertion until the string or the
hook is fixed. Copy is English, matching the rest of the app's UI.

| Hook | Copy |
| :-- | :-- |
| `data-print="label-realized"` | `Realized P&amp;L — cumulative since inception` |
| `data-print="rounding-note"` | `Figures are rounded for display from full-precision calculations. The tax report CSV carries the unrounded values.` |
| `data-print="empty-note"` | `No holdings in this portfolio on {date}.` |
| `data-print="splits-note"` | `Split-adjusted history is as recorded on {date}; late-recorded splits can change past figures.` |
| `data-print="run-context"` | `{portfolio name} · {base currency} · {strategy}` plus ` (threshold {threshold_days}d)` only when strategy is `HYBRID` |

The rounding note is conditional in one case: a base currency whose `Intl` rendering is already
exact for the values in hand still prints it, because the payload's precision is not the
currency's precision. Simpler to always print it than to compute when it would be redundant.

### Empty portfolio

`analytics.rs:45-51` returns `assets: []`, three zero strings and **no** `currency`,
`strategy` or `threshold_days`. Goal 7 is the branch: the component checks
`taxSummary.assets.length === 0` and renders `data-print="empty-note"` alone — no headline
figures, no run-context line, no holdings table. The existing holdings table already makes the
same branch for the same reason (`PortfolioDetail.tsx:238`), so this is a repo convention, not
an invention. Verification layer 1 asserts it with an empty-`assets` fixture.

### Money: a display artifact, declared

The statement renders payload numbers through `formatters.ts`, which quantises at display:
to 2 dp for a fiat base currency, and to between 2 and 8 dp for a crypto base currency
(`formatters.ts:10-11` gives `BTC`/`ETH` `maximumFractionDigits: 8`). It does not attempt to
be exact, and it says so on the page.

*   **Why not preserve the decimal strings?** The render path throws them away one layer below
    where they would be preserved. `formatCurrency` is `const v = toNumber(value)`
    (`formatters.ts:4`) and `toNumber` is `Number(v)` (`decimal.ts:20-23`), so
    `formatCurrency("1234.565")` and `formatCurrency(1234.565)` are byte-identical. A
    string-preserving normalizer changes nothing observable while adding a second shape for one
    payload and a drift duty to go with it. See Alternative C1.
*   **Why not make the page exact?** Exactness is the filed artifact's job; a page would then
    print `1234.56500000000000000000` to a person. See Alternative C2, which also covers the
    cost of a decimal rounder and the `sellPnlCsv.ts:61-65` precedent.
*   **The double, with its limit.** A double only diverges from exact decimal rounding once the
    input carries more significant digits than a double holds (~15-17), and the engine's
    products do: `tax_engine.rs:414` is `(tx_qty * tx_price + tx_fee) / tx_qty` and the FX
    multiplications at `:418`, `:422`, `:433` have no scale control, so `1234.56499999999999999999`
    is a reachable value and rounds to `1,234.57` through a double where exact half-up gives
    `1,234.56`. The probability in practice is negligible; quantisation is the divergence that
    actually happens, which is why Goal 3 declares that. Everything in this document's money
    claims is scoped to inputs of 15 significant digits or fewer; beyond that the residual is
    the truncation above and it is not claimed to be zero.
*   **What the payload supplies, field by field.** The three headline figures are
    `total_portfolio_value`, `total_realized_pnl` and `total_unrealized_pnl`; the holdings rows
    come from `assets[]`, whose per-lot fields are named in the parent design. The run context
    is `strategy`, `threshold_days` and `currency`, all three echoed by the server
    (`analytics.rs:98-100`). `normalizeTaxSummary` (`decimal.ts:63`) currently keeps only
    `currency`, so this slice adds two fields to the hand-written type and two lines to the
    normalizer — additive, moves no figure.
*   **Which currency formats the money, and which prints in the header.** Money is formatted
    with the payload's `currency`. The header line prints the **portfolio object's** base
    currency, because the empty-portfolio branch (`analytics.rs:45-51`) returns no `currency`
    key at all and `normalizeTaxSummary` then defaults it to `USD` — so an empty EUR portfolio
    would otherwise print "USD". These are not two competing sources: the non-empty branch
    returns `currency` **as** `portfolio.base_currency` (`analytics.rs:99`), so they are the
    same string whenever there are holdings to format, and they can only differ when there are
    none — and on that branch the statement prints no money at all (see Empty portfolio).

### What "splits" and the strategy line mean

Two pieces of this project's vocabulary are load-bearing on the printed page and are not general
engineering terms.

*   **Splits.** Stock splits, auto-detected and persisted by the `tax-summary` handler itself
    (`analytics.rs:137-140`, `sync_splits_for_asset` per asset) before it computes anything. So
    "splits are as recorded at that moment" means: the split rows in the database when the
    request ran, which the handler may have just added from the provider. The caveat exists
    because a split can be recorded long after it happened, which silently changes history —
    the open hazard tracked as issue #50. Printed copy, verbatim:
    *`Split-adjusted history is as recorded on {date}; late-recorded splits can change past figures.`*
*   **`HYBRID` and `threshold_days`.** The engine supports `FIFO`, `LIFO` and `HYBRID`
    (`tax_engine.rs:396`, `:453`). `HYBRID` consumes lots held for **`threshold_days` or
    fewer** first — the short-term bucket — and `threshold_days` is that short/long boundary in
    days. It is meaningless for `FIFO` and `LIFO`: `term_for` (`tax_engine.rs:645-649`) returns
    `None` for anything but `HYBRID`. That is why Goal 4 prints it only for `HYBRID`; the
    on-screen precedent is `SellPnlModal.tsx:210`.

### Palette inversion

The palette is variable-driven (`index.css:3-31`), and components read it through the
variables — including inline styles such as
`style={{ color: 'var(--text-secondary)' }}` in `SellPnlModal.tsx:206`. That is what makes
this cheap: `print.css` overrides the variables on `:root` inside `@media print` and most
of the app flips at once, inline styles included.

`frontend/src/print.css` (new, flat beside `index.css` — the repo has no `src/styles/`
directory and inventing a one-file directory is not worth it) is imported from `main.tsx`
after `index.css`. It is imported second so it wins the cascade at equal specificity; that is
source order doing the work, not the file boundary — a `@media print` block appended to
`index.css` would win just as reliably. The separate file is for organisation, and the print
palette sits beside nothing rather than beside the palette it inverts, which is the one thing
the split costs.

It does six things, in this order:

1.  `@page { size: A4; margin: 14mm }`. Letter users change it in the print dialog; the
    choice is stated rather than hidden. `@page` has no scope, so it applies to a print
    started from any tab — see the scoping note below.
2.  **Hide the shell, not a list of things inside it.** Two rules, in this order:

    ```css
    .app-container > *:not(.main-content) { display: none }
    .main-content > *:not([data-print="statement"]) { display: none }
    ```

    The first rule removes the sidebar (`<aside className="sidebar">`, `App.tsx:379`) and
    every other direct child of `.app-container` — which includes the auth-failed alert at
    `App.tsx:359-376`, a `position: fixed` full-width red banner that an enumerated hide list
    would print on every page. The second leaves only the statement in the content column, so
    the header row (`<div className="header-row">`, `App.tsx:458`) and any chrome added there
    later disappear without anyone remembering a list. An allowlist of what to hide drifts
    every time someone adds chrome, in the same direction, silently; the inclusive form cannot
    miss.

    **Two structural preconditions, because both rules use the child combinator `>`.** The
    statement's root element must carry `data-print="statement"` **and be a direct child of
    `.main-content`** — not wrapped in a layout `<div>` or `<section>`. Wrap it and rule 2 hides
    the wrapper, so the printed page comes out blank while the on-screen tab looks correct. The
    class names `.app-container` and `.main-content` are therefore load-bearing for printing, not
    cosmetic; renaming them in a later refactor silently breaks the print path. The smoke's
    visibility assertion is what catches this, and only when someone runs it.


    **The selectors are named here on purpose.** The parent design's file list cited
    `header.app-header` at `App.tsx:385-396`; that element does not exist. A gate that greps
    for a selector nobody wrote is worse than no gate.
3.  Collapse the layout: `.app-container` is a `260px 1fr` grid (`index.css:93-98`); in
    print it becomes one column, or the page prints with a 260 px gutter of nothing.
4.  Invert: `--bg-dark`, `--bg-card`, `--border-color`, `--text-primary`,
    `--text-secondary`, `--text-muted` to print values; `.glass-card` loses
    `backdrop-filter`, `box-shadow` and its translucent background. **Which of these carries
    Goal 6:** legibility comes entirely from the **text** overrides (`--text-primary`,
    `--text-secondary`, `--text-muted`), because text is always painted and does not depend on
    the background-graphics setting at all. The background and border overrides are
    belt-and-braces: they matter only for a reader who *has* enabled background graphics, and
    without them that reader gets dark text on the dark app background. So Goal 6 holds either
    way, and the two groups are not interchangeable — a build that ships only the background
    overrides prints an empty-looking page. Sign is carried by the number, never by colour:
    `formatCurrency` already renders negatives as `-$1,234.56`, so a greyscale printer loses
    nothing.
5.  Break rules: `tr { break-inside: avoid }`, and the headline block never splits.
6.  `[data-no-print] { display: none }` — the opt-out hatch for anything inside the printed
    subtree. Today nothing uses it: the Print control sits outside the subtree and rule 2
    already hides it. It is specified so a later addition can opt out without restructuring
    the tree.

**Scoping the inversion.** The rules above live under `@media print` and are not scoped to the
statement tab, so Ctrl-P from the dashboard after this ships produces a light-on-dark page
with the sidebar hidden and the dashboard printed — better than today's white-on-white, but not
a designed output. Some backgrounds are not variables and will not flip: `.badge-crypto`
(`index.css:334`) and `.data-table tr:hover td` (`:252`). (`.sidebar` is also hardcoded
`rgba(10, 15, 28, 0.95)` at `:101`, but rule 1 hides it, so it cannot print badly either way
and is not a residual.) The decision recorded here is that **printing from another tab is out
of scope and stays that way**: the statement is the tab you print from, and the
statement tab is the only one the smoke exercises. If that ever becomes wrong, the fix is a
body attribute set by the statement tab and every print rule scoped under it — one attribute,
not a rewrite.

### Verification, and what it cannot cover

Three layers, weakest first, with the gap named at each:

1.  **Vitest component test** (`frontend/src/test/components/PortfolioStatement.test.tsx`).
    jsdom does not apply print CSS, so this proves structure and formatting only.

    *Formatting* is asserted by a **golden**: a fixture carrying a half-cent value
    (`1234.565`) and an expected literal rendered string in the test. Anything of the form
    "the printed figure equals the payload value rendered through `formatters.ts`" cannot fail
    for a component that calls `formatCurrency`, and deriving the expected value by rounding
    the payload with a double re-runs the renderer's own path — the same unfailable-test
    failure this document uses to reject the grep gate (Alternative B). No assertion here may
    compare two normalizers, for the same reason.

    *Structure*: the statement root carries `data-print="statement"`; the printed subtree
    contains no `button`, `input`, `select` or `svg`; all three headline figures appear; the
    the `rounding-note`, `splits-note` and `label-realized` hooks from *On-page copy* are each
    present with their specified text; the run-context line carries the payload's echoed
    `strategy` and `threshold_days`, with `threshold_days` absent for a non-HYBRID fixture; and
    an `assets: []` fixture renders the `empty-note` line and none of the headline figures. The mock at
    `handlers.ts:121` is reused rather than registered twice: the existing
    `/api/portfolios/:id/tax-summary` handler gains a branch on the portfolio id, so the
    statement fixture and the sell-P&L fixture answer different ids and neither test disturbs
    the other. Its money fields become decimal strings to match what the server actually sends
    (`db_types.rs:31`), and it gains `strategy`, `threshold_days` and `currency`. The
    registration-order comment at `handlers.ts:130-131` still applies — MSW takes the **first**
    match, so a specific route registered below a wildcard is unreachable. Branching inside the
    existing handler sidesteps that ordering hazard rather than adding to it, which is the second
    reason to branch rather than register a second handler.
2.  **Playwright smoke** (`frontend/e2e/statement.smoke.mjs`, following
    `sell-pnl.smoke.mjs`). `page.emulateMedia({ media: 'print' })`, then assert the
    statement is **visible** — non-zero box, dark computed text color on a light background,
    which is the check that actually catches the dark-theme failure, as a computed-style check
    rather than a screenshot — and that the shell is gone (`.sidebar` and the auth alert
    compute `display: none`). Asserting visibility is the primary check; asserting an
    enumerated list of hidden selectors would only re-state a rule that already cannot miss.
    Also assert no horizontal overflow, and that `page.pdf()` is non-empty and one page for
    the two-row fixture — **for the fixture only**, with no claim about larger portfolios
    (Decided item 2).

    **Local step, not a gate.** `e2e/run.sh` hardcodes `node e2e/sell-pnl.smoke.mjs` and must
    be parameterised first; `ci.yml` has no browser job; both are issue #45. The reason is a
    choice, not an impossibility: CI runs `ubuntu-latest`, where `npx playwright install
    chromium` is one line and `emulateMedia`/`page.pdf()` need no display server. It stays
    local because `e2e/run.sh:3-16` is not browser-only — it compiles and starts the Rust API
    against a throwaway SQLite file and starts Vite — so the job costs a full cargo build and
    needs seeded data, and because a `continue-on-error` job nobody reads converts
    "unverified" into "verified and therefore fine" by another door. The real alternative is a
    blocking browser job that also runs the existing sell-P&L smoke, which is #45's scope.
    The assertions above are handed to #45 so they land with a job that can execute them.

    **Prerequisites this document must state, because it cannot inherit them silently.** The
    smoke drives a real browser at a real backend, and every `/api` route takes `AuthedUser` —
    the same fact the Problem section uses to argue the adviser cannot log in. So the run needs
    `AUTH_MODE=disabled`, which is compiled out of release builds (`auth/mod.rs:97-101`).
    **`frontend/e2e/run.sh` does not set it, and there is no `backend_rust/.env`**: with
    `AUTH_MODE` unset the server asks for OIDC (`auth/mod.rs:85`) and bails naming four missing
    variables (`:132-145`), so `wait_for backend` burns its whole 420-second budget and the
    smoke dies with "backend did not come up". The last commit to touch `run.sh` predates the
    auth work. So "follow `sell-pnl.smoke.mjs`" is not a safe instruction on its own: the
    harness it points at does not currently start. Two things follow, and both belong to #45
    rather than to this slice — `run.sh` gains `export AUTH_MODE=disabled` beside its existing
    `DATABASE_URL` export, and the statement smoke seeds through the REST API the way
    `sell-pnl.smoke.mjs:37-57` already does, which needs no browser login because the seeding
    is server-side. The fixture is two holdings in one portfolio, created by the smoke, with
    the strategy passed explicitly so the printed run context is known rather than defaulted.

3.  **A human with a print preview.** `docs/auth-testing.md` §5 is already the manual
    browser walkthrough for what CI cannot check; the statement's print preview joins that
    list.

### Sequencing

One slice. There is no useful smaller cut: the page without the print stylesheet is a
dashboard with a different heading, and the print stylesheet without the page has nothing to
style.

Two column terms, since both are used as terms of art throughout: **"reaches network in
tests"** means a test contacting a real backend or the Yahoo provider — intercepted fetches
through `test/mocks/handlers.ts` count as not reaching it; **"moves an existing figure"** means
any number already displayed somewhere on screen changes value for the same portfolio and the
same data.

| Slice | Ships | Reaches network in tests? | Moves an existing figure? |
| :-- | :-- | :-- | :-- |
| 1 | Statement tab, print stylesheet, two payload fields surfaced, component test, local smoke | no | no |

## Target Files

#### 1. `frontend/src/components/PortfolioStatement.tsx` (Create)

*   **Type / Scope:** the printable view.
*   **Proposed Changes:** renders the summary: headline block (cumulative
    realized since inception, unrealized as of today, portfolio value), the rounding line
    Goal 3 requires, and one row per holding (Decided item 1). No asset-subset filter — the
    only arithmetic that would make a filtered headline honest is the arithmetic renderers
    are forbidden from doing. Takes no date parameter. Derives no totals. No `button`,
    `input` or hardcoded color inside the printed subtree; the Print control sits outside it
    with `data-no-print`.

#### 2. `frontend/src/print.css` (Create) and `frontend/src/main.tsx` (Modify)

*   **Type / Scope:** the print stylesheet, imported after `index.css` so it wins the
    cascade at equal specificity.
*   **Proposed Changes:** the six rules above, with the selectors and the two structural preconditions named exactly as in
    Palette inversion.

#### 3. `frontend/src/utils/decimal.ts` (Modify) and `frontend/src/types.ts` (Modify)

*   **Type / Scope:** surface the two run-context fields the payload already sends.
*   **Proposed Changes:** `normalizeTaxSummary` keeps `strategy` and `threshold_days`
    (`decimal.ts:63` currently keeps `currency` only) and `TaxSummary` in `types.ts` gains
    both. Additive; no existing figure moves. There is **no** second normalizer and no
    string-typed mirror type — see Alternative C1 for why.

#### 4. `frontend/src/App.tsx` (Modify)

*   **Type / Scope:** the fifth nav item and the tab branch.
*   **Proposed Changes:** renders the statement from the `taxSummary` state
    `fetchPortfolioData` already sets, guarded the way the sibling tab guards
    (`activeTab === 'portfolios' && taxSummary`, `App.tsx:483`) so a refresh in flight cannot
    print zeros, and it keys on the selected portfolio rather than on `taxSummary` merely being
    non-null — `setTaxSummary` runs once (`App.tsx:114`) and is never cleared, so a portfolio
    switch leaves the previous portfolio's summary in state until the new fetch resolves, and a
    statement rendered from it would print B's numbers under A's name. The header's base
    currency comes from the portfolio object rather than the payload, for the empty branch only:
    `analytics.rs:45-51` returns no `currency` key and `normalizeTaxSummary` defaults it to
    `USD`, so an empty EUR portfolio would otherwise print "USD". Money itself formats with the
    payload's `currency`; the two are the same string whenever there are holdings
    (`analytics.rs:99`).

#### 5. `frontend/src/test/components/PortfolioStatement.test.tsx` (Create) and `frontend/src/test/mocks/handlers.ts` (Modify)

*   **Type / Scope:** structural tests and the mock branch.
*   **Proposed Changes:** the assertions listed under Verification layer 1. The mock at
    `handlers.ts:121` must also be corrected: it returns `total_realized_pnl: 0` as a JSON
    **number** while the backend always sends strings (`analytics.rs:95-97`,
    `db_types.rs:31`), so the fixture has to carry decimal strings and the two echoed
    context fields, or the statement is tested against a shape the server never emits.

#### 6. `frontend/e2e/statement.smoke.mjs` (Create), `frontend/e2e/run.sh` (Modify), `frontend/package.json` (Modify)

*   **Type / Scope:** the browser smoke, and the parameterisation it needs.
*   **Proposed Changes:** as in Verification layer 2. `run.sh` currently hardcodes
    `node e2e/sell-pnl.smoke.mjs`; it gains a target argument defaulting to the existing
    smoke, and `package.json` gains `e2e:statement`. **Not wired into CI** — see Verification
    layer 2 and issue #45.

#### 7. Deleted — no print-CSS grep gate

Carried from the parent design. A grep asserting a stylesheet mentions some selectors goes
green while the page still prints badly. The Playwright smoke is the honest check; a grep
that can pass while the feature is broken is a gate that teaches people to ignore gates.

## Risks

| Risk | Evidence | Mitigation |
| :-- | :-- | :-- |
| **The print path ships unverified.** No CI browser job; the smoke is a local step. The reason is a choice about CI cost, not an impossibility — Verification layer 2 also records that the existing e2e harness does not currently start. | `ci.yml` has no browser job; issue #45 open; `run.sh` sets no `AUTH_MODE` | Selectors and smoke assertions are named in this document and handed to #45, so the check exists and is specific the moment that job lands. The manual print preview is listed in `docs/auth-testing.md` §5. |
| **Palette override misses a hardcoded color.** | `SellPnlModal.tsx:132` uses inline `background: '#121929'` | The statement uses no hardcoded colors; the smoke asserts computed text color, which catches the failure regardless of cause. |
| **A printed figure differs from the CSV's.** | The CSV is full-precision (`docs/superpowers/reviews/2026-11-05-sell-pnl-breakdown.md:18`) and `formatCurrency` quantises to 2 dp (`formatters.ts:4`) | Goal 3: the page declares its rounding, so the difference is stated rather than discovered by an adviser |
| **A third renderer of one payload, with nothing comparing it to the second.** | `PortfolioDetail.tsx:238-292` renders the same `taxSummary.assets`; `Dashboard.tsx:40-45` renders the same headline figures | Accepted knowingly (Alternative D). The statement is a leaf with no interactivity and formats money with the payload's `currency`, where the holdings table uses `portfolio?.currency \|\| 'USD'` (`PortfolioDetail.tsx:261`). If the two tables ever need to agree, the check is a comparison test between them, and it is not written here. |
| **A4 on Letter hardware.** | `@page { size: A4 }` | Stated in the dialog, not hidden. The owner's portfolios are EUR-denominated, which is a hint about likely paper preference and nothing more; the print dialog overrides either way. |
| **Statement read as a quarterly figure.** | The undated endpoint has no window | Goal 1's label requirement: "cumulative since inception" is on the page, not implied. |
| **Empty or failing portfolio prints zeros.** | `analytics.rs:45-51` returns zeros with no `currency` | Goal 7 and the Empty portfolio section: that branch prints the `empty-note` line and no figures. Header currency comes from the portfolio object. |
| **A stale payload prints under the wrong portfolio name.** | `setTaxSummary` is called once (`App.tsx:114`) and never cleared, so switching portfolios leaves the previous portfolio's summary in state until the new fetch resolves | The tab branch keys on the selected portfolio, not merely on `taxSummary` being non-null: render only when the summary belongs to `selectedId`, otherwise render nothing. A printed page with B's numbers under A's heading is the one failure this artifact cannot absorb quietly, and it is worse than the zeros case because it looks right. |

## Decided during drafting

Three items were open when this document was first written. They are settled here, and the
reasons are recorded because each one trades something away.

1.  **The statement lists holdings.** The headline figures are the point, and a 40-row
    table is a second page and a second set of numbers to reconcile against the CSV. It
    lists them anyway: a statement without holdings is a receipt, and the owner's use case
    is handing something to a person who will ask "which positions?". The reconciliation
    cost is real and is bounded by Goal 3 — every printed row is a payload value quantised
    once at display, so a row cannot disagree with the CSV except by the rounding the page
    declares. What the rows can do is run to a second page, which is a layout fact, not a
    correctness one.
2.  **The smoke asserts one page for its own fixture and says nothing about larger
    portfolios.** One page for a two-row fixture is checkable. A bound that holds for 50
    holdings is a layout claim nobody has measured, and an assertion that fails on the
    owner's real portfolio but passes in CI is an assertion that teaches people to ignore
    the smoke. The page-break rules still ship (`tr { break-inside: avoid }`); they are
    simply not claimed to produce a page count.
3.  **The strategy line prints what the server echoed.** An earlier draft of this document
    claimed the payload does not carry `strategy` or `threshold_days`, and built a hedging
    label and a risk row on that claim. It was wrong: `analytics.rs:98-100` returns
    `strategy`, `threshold_days` and `currency`, and the route passes the query values
    straight to `calculate_lots`, so the echoed value *is* the value used. The only gap was
    that `normalizeTaxSummary` dropped two of the three fields. Nothing to hedge, and
    nothing to decide beyond surfacing them — the wording precedent is already on screen at
    `SellPnlModal.tsx:210` (`realized · FIFO (threshold 30d)`). Recorded because the false
    claim was load-bearing in three places and a reader should be able to see what was
    removed and why.

## Alternatives Considered

### Alternative A: server-generated PDF

*   **Pros**: exact pagination, no browser print dialog, a real artifact to hash and file.
*   **Cons**: a new Rust dependency with a font-file requirement (`printpdf`, `genpdf`) or a
    headless browser on the server. The app runs in a single-replica pod with a 256 Mi memory
    limit (`k8s/app-deployment.yaml:78`, `replicas: 1` at `:9`) — PDF generation or a headless
    browser is the kind of work that turns a working pod into an OOMKill.
*   **Reason for rejection**: the statement is read or emailed, not filed or signed. The CSV
    is the filed artifact, and when the tax report ships its CSV is specified to be exact.
    Browser print-to-PDF reuses the existing
    formatters, so the numbers look like the rest of the app for free.

### Alternative B: a print-CSS grep gate in CI

*   **Pros**: something runs on every push; cheap.
*   **Cons**: it asserts that a stylesheet mentions selectors, not that the page prints. It
    passes on a page that prints white-on-white.
*   **Reason for rejection**: a gate that can pass while the feature is broken is worse than
    an honest local step, because it converts "unverified" into "verified and therefore
    fine". The Playwright smoke is the gate; it is waiting on issue #45, which is the same
    blocker as the existing sell-P&L smoke.
*   **Scope note, because this rejection is easy to over-apply.** What is rejected here is a
    grep over a *stylesheet*. A different check was considered and is not rejected by the
    argument above: a grep over `PortfolioStatement.tsx` for a hardcoded color literal,
    which fails precisely when the "no hardcoded colors" convention breaks, and which the
    repo already runs in this shape with its silent-no-match failure mode solved
    (`backend_rust/scripts/check_access_gate.sh`). It is out of this slice only because the
    convention it protects is one file wide and the smoke would catch the same breakage. If a
    print gate is ever added, that is the check worth adding, not this one.

### Alternative C: make the printed page exact

Two variants were considered for making the page match the CSV to the decimal rather than
declaring a rounding.

*   **C1 — a string-preserving normalizer for the statement.** This was the first draft's
    choice. **Rejected on verification, not on taste:** `formatCurrency` calls `toNumber`
    (`formatters.ts:4`), so the preserved string is converted to a double one layer below the
    boundary it was preserved to protect, and the printed output is byte-identical either way.
    It bought a second payload shape and a drift duty for no observable change.
*   **C2 — a string-preserving money formatter in `formatters.ts`,** used by the statement and
    eventually by everything. The only variant that would actually work. **Rejected because
    exactness is the wrong property for this artifact:** a page read by a person should not
    print `1234.56500000000000000000`, and the filed artifact — the tax report CSV, once it
    ships — is the one meant to be exact. It also means a decimal rounder right about scale,
    sign, `-0` and exponent input. `sellPnlCsv.ts:61-65` is a partial precedent: it handles the
    exponent case and documents the `1e21` `toFixed` hazard, but it shapes values already
    through `Number()`, so it is a starting point rather than the utility. And if the formatter
    lands inside `formatCurrency` it changes what every existing screen prints for >2-dp
    values, which this slice promised not to do.
*   **If exactness is ever required**, C2 is the shape to take, as its own change with its own
    figure note. Recorded here so the next person does not re-derive C1.

### Alternative D: a print mode over the view that already renders this payload

*   **Pros**: no third renderer. `PortfolioDetail.tsx:238-292` already renders a holdings
    table from `taxSummary.assets` — symbol, type, shares, average cost, price, market value,
    realized P&L, unrealized P&L, ROI — which is Decided item 1's row set plus an Actions
    column, and `Dashboard.tsx:40-45` already renders the headline figures from the same
    payload. The document's own description of the statement ("a dashboard with a different
    heading") is an admission that this option was available.
*   **Cons**: the existing table is exactly what Goal 2 forbids. It has buttons, expanders, an
    Actions column, and inline `var(--color-success)` colors on the P&L cells
    (`PortfolioDetail.tsx:286-292`); it takes its currency from `portfolio?.currency || 'USD'`
    (`:261`) rather than from the payload. Making it print well means suppressing most of it
    and re-deriving what the leaf component gets for free.
*   **Reason for rejection**: a dedicated leaf is the cheaper honest artifact. But the choice is
    recorded as what it is — **this slice adds a third renderer of one payload** — and the
    drift risk it creates is in the Risks table, not hidden here. The two tables will disagree
    about columns, about currency source, and about sign and rounding conventions unless
    something keeps them honest; the smoke and the golden are the only things currently named
    to do that, and neither compares the two tables.

### Alternative E: the same route serves server-rendered HTML

*   **Pros**: the parent design already commits to `?format=csv` on `tax-summary`, "served
    where the exact decimals live", on the grounds that a total which must equal `tax-summary`
    to the decimal should not be re-passed through a double on its way to a file. That
    argument applies verbatim here. A `?format=print` variant with a small inline light
    stylesheet dissolves the palette inversion, the rounding declaration, the adviser-access
    problem and the whole print-stylesheet section at once, and produces a URL the owner can
    bookmark and mail.
*   **Cons**: a templating dependency or hand-built HTML in Rust; a second rendering technology
    in a repo that has none today; it is the backend change Goal 5 refuses; it is blocked on
    the parent design's route work and therefore on the pricing layer, which destroys the one
    property that makes this slice worth doing first; and it cannot reuse `formatters.ts`, so
    currency grouping drifts from the rest of the app.
*   **Reason for rejection**: all four costs, especially the dependency on the pricing layer.
    Recorded because Alternative A rejects only the PDF variant, and a reader would otherwise
    conclude server rendering had been considered when it had not.

### Alternative F: a copy-to-clipboard text block instead of a printed page

*   **Pros**: the adviser cannot log in (see Problem), so the artifact leaves the app either
    way. A plain-text block — run context, the headline figures, the holdings rows — pasted
    into an email body needs no print stylesheet, no palette inversion, no `@page`, no
    A4/Letter question, no page-count question, and no CI gap, and it is the only variant that
    survives an adviser replying inline. The app already ships text artifacts
    (`utils/download.ts`, `utils/sellPnlCsv.ts`).
*   **Cons**: it cannot carry a 40-row table legibly, and it is not something an adviser files.
*   **Reason for rejection**: Decided item 1 is an explicit owner decision that the artifact is
    a document. A text block is a different product with a different audience, and shipping
    both would mean two formats to keep in agreement for a use case that has one artifact today.

### Alternative G: `?view=statement` as a poor man's deep link

*   **Pros**: `App.tsx:58` already reads `new URLSearchParams(window.location.search)` and
    `App.tsx:347-353` rewrites the query string, so a query parameter follows an existing
    convention in the exact file cited as lacking routing — no router, no dependency, and the
    smoke could drive it with `page.goto('/?view=statement')` instead of a click path.
*   **Cons**: `selectedId` is component state and is not in the URL either, so a bookmarked
    `?view=statement` reopens the default portfolio. It buys half a deep link and adds a second
    navigation mechanism alongside the one that works.
*   **Reason for rejection**: half a deep link is not worth a second navigation mechanism. If
    linkability is ever wanted it is wanted for portfolio selection first, and that is a router
    conversation this slice declines.

## Verification

*   `npm test`, `npm run lint`, `npm run typecheck` from `frontend/`.
*   `npm run e2e:statement` from `frontend/` — **local step, not a gate**. Needs a chromium
    binary and fonts; on NixOS,
    `nix-shell -p chromium -p liberation_ttf --run './e2e/run.sh statement'`.
*   Manual: open the statement, print preview, confirm no sidebar, no nav, no print button,
    dark text on white, and that the headline figures match the dashboard. Added to the
    manual browser walkthrough in `docs/auth-testing.md` §5.
*   Cross-check **once the tax report ships** — it does not exist yet, and its own date
    semantics are still open in the parent design: same portfolio, same moment, statement's
    cumulative realized equals the CSV's cumulative realized column rounded the way
    `formatters.ts` rounds it, and the page carries the line saying it was rounded. Until
    then the only shipped equality is the golden in layer 1, evaluated against the payload.
