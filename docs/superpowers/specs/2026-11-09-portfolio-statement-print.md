# Portfolio Statement (Printable)

*   **Author:** javierarrieta, with EGM review support
*   **Status:** Proposed
*   **Date:** 2026-11-09
*   **Depends on:** `2026-11-09-pricing-layer-truthfulness.md` (dated FX — a statement that
    converts at the wrong date is worse than no statement). **Related:**
    `2026-11-09-tax-report-and-statement-design.md` (the tax report; this document was
    split out of it).
*   **Origin:** split out of the tax report design during its EGM review
    (`docs/superpowers/reviews/2026-11-09-tax-report/`), stage 8 scope judgement.

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
*   **The printed page must say what it rounded.** The backend sends money as decimal
    strings (`backend_rust/src/db_types.rs:31`, `serializer.serialize_str(&d.to_string())`)
    at full precision, and the tax report CSV is specified to be written at that same full
    precision by standing repo decision
    (`docs/superpowers/reviews/2026-11-05-sell-pnl-breakdown.md:18`,
    "full precision, no quantisation"; per-field rounding is called "actively harmful" at
    `:50`). The engine multiplies by the FX rate with no `set_scale` anywhere
    (`tax_engine.rs:418`, `:422`, `:433`), so a realized figure genuinely carries more
    decimals than a page should show. `normalizeTaxSummary` in
    `frontend/src/utils/decimal.ts` runs every money field through `num()`, which is
    `Number(v)`, and `formatCurrency` runs it through `toNumber` again
    (`formatters.ts:4`) before quantising to 2 dp via `Intl`. So a printed figure and the
    CSV's figure for the same holding are **already guaranteed to differ** — not by a
    floating-point accident but by design, because one is quantised and the other is not.
    A page that prints `1,234.57` next to a CSV cell reading `1234.56500000000000000000`
    and claims to be the same number is the defect. The fix is to say the rounding out loud
    on the page, not to pretend the page is exact.
*   **There is no router, and therefore no linkable statement.** `App.tsx:36` is
    `useState(DEFAULT_TAB)` and the nav is four `<li className="nav-item">` elements
    (`App.tsx:385-396`). Adding a router for one page is not worth it and is a non-goal.
    The honest consequence is that **a printed page cannot be re-opened by URL and the
    statement is not bookmarkable**, and this document stops pretending that is a problem it
    intends to solve. A query parameter was considered and rejected: `App.tsx:58` already
    reads `new URLSearchParams(window.location.search)` and `App.tsx:347-353` rewrites it, so
    `?view=statement` would follow an existing convention with no dependency — but
    `selectedId` is component state and is not in the URL either, so a bookmarked
    `?view=statement` reopens the wrong portfolio. It buys half a deep link, adds a second
    navigation mechanism next to the one that works, and the smoke can drive the real
    mechanism directly. If linkability is ever wanted, it is wanted for portfolio selection
    first, and that is a router conversation this slice declines.
*   **Nothing can prove it prints, and the reason is a workstation fact, not a CI law.**
    `.github/workflows/ci.yml` has no browser job. Issue #45
    (*Run the sell P&L Playwright smoke in CI*) is open. This slice therefore **ships
    unverified by design** — see Risks and Verification layer 2. That phrase was inherited from
    the parent design and it needs the correction recorded there: CI could run this smoke
    today with a one-line chromium install; the reasons it does not are the cargo build the
    harness drags in and the emptiness of a job allowed to fail. The document names its
    selectors and its smoke assertions here rather than leaving them to implementation time,
    so the check is specific the moment #45 lands.

*   **The person the statement is for cannot log in.** Every `/api` route goes through
    `AuthedUser` plus a `require*` call, and `apiFetch.ts:22-26` sends an unauthenticated
    caller to `/auth/login`. An adviser has no account and will never get one from this
    slice. So the artifact necessarily leaves the app, and "print it from the browser" is not
    a complete story: the owner prints, then forwards. That is fine, but it means the page
    must stand on its own once detached — which is what Goal 4's provenance block and Goal 3's
    declared rounding are for. The cheaper exit (a copy-to-clipboard text block pasted into an
    email body) is considered and rejected under Alternative F.

## Goals

*   **Goal 1 — the P&L figures, honestly labelled.** Cumulative realized P&L and unrealized
    P&L for the selected portfolio, with realized labelled as **cumulative since
    inception**; portfolio value prints alongside them and needs no such label. The undated
    `tax-summary` endpoint has no window, so a statement cannot show a quarterly realized
    figure; an unlabelled number next to a quarterly tax form is how a wrong filing starts.
*   **Goal 2 — the printed page is a document.** No sidebar, no navigation, no buttons, no
    inputs, no charts, no error banner. One column. `@page { size: A4; margin: 14mm }`.
*   **Goal 3 — the printed numbers are the payload's numbers, quantised once and declared.**
    The statement is a **display artifact**. Every money value is a payload value, rendered
    through `frontend/src/utils/formatters.ts`, quantised at display and nowhere else — to
    the cent for a fiat base currency, to `formatters.ts`'s crypto precision for a `BTC` or
    `ETH` base currency, which is the same rule every other screen already follows and is
    stated here because "rounded to the cent" would be a lie on a BTC-denominated page.
    The statement performs no arithmetic: it sums nothing, differences nothing, and never
    re-derives a total. The payload itself carries full precision (`db_types.rs:31`), so the
    page carries a visible line saying its figures are rounded from the payload's own
    precision — the difference is declared rather than left for the adviser to discover.
    When the tax report ships, its CSV is expected to hold the payload's full precision, and
    the statement's cumulative realized figure should then equal that column **rounded the
    same way `formatters.ts` rounds it**. That cross-check is conditional on the tax report
    existing; the test that ships with this slice is the golden described under Verification,
    which needs no second rounder to evaluate.
*   **Goal 4 — the page states its own provenance.** Portfolio name, base currency, the
    strategy and `threshold_days` **as the server echoed them** (`analytics.rs:98-100`), the
    generation date, and the standing caveat that splits are as recorded at that moment.
    `threshold_days` prints only for `HYBRID`, following the on-screen precedent at
    `SellPnlModal.tsx:210`; next to `FIFO` it is an unused parameter and printing it would
    present one as part of the method. The empty-portfolio branch at `analytics.rs:45-51`
    echoes **none** of the three fields, so the run-context line is omitted there rather than
    rendered from `str()`/`num()` defaults — a line reading "realized · (threshold 0d)" would
    be a fabricated run context on the one branch already cited for the currency problem.
*   **Goal 5 — no backend change in this slice.** No new route, no new field, no new figure on
    any existing endpoint, no OpenAPI regeneration. This is a **sequencing** boundary, not a
    standing principle: the parent design already rewrites this same handler's response, which
    is why the committed OpenAPI body is empty, and will add `from`/`to` to it. If this slice
    needed the backend it would mean the statement had drifted out of scope, and it should
    stop. The cost of the boundary is stated: the run context the page prints is a subset of
    what the parent design will eventually carry anyway, so echoing `strategy`,
    `threshold_days` and `currency` is plumbing the sibling change does regardless. The
    counter-argument for keeping the boundary is that any response-shape change lands in
    `docs/openapi/openapi.json`, which CI diffs byte-for-byte, so it is not a quiet two-field
    change — and the statement is the one slice of this programme not blocked on the pricing
    layer.
*   **Goal 6 — the palette inversion does not depend on a browser setting.** It must not
    rely on "background graphics" being ticked. Text becomes dark because the stylesheet
    says so, not because the browser kept a background.

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

### Money: a display artifact, declared

The statement renders payload numbers through `formatters.ts`, which quantises at display:
to 2 dp for a fiat base currency, and to between 2 and 8 dp for a crypto base currency
(`formatters.ts:10-11` gives `BTC`/`ETH` `maximumFractionDigits: 8`). It does not attempt to
be exact, and it says so on the page.

*   **Why not preserve the decimal strings?** Because the render path throws them away one
    layer below where they would be preserved. `formatCurrency` is
    `const v = toNumber(value)` (`formatters.ts:4`) and `toNumber` is `Number(v)`
    (`decimal.ts:20-23`), so `formatCurrency("1234.565")` and `formatCurrency(1234.565)`
    are byte-identical. A string-preserving normalizer would change nothing observable on
    the printed page while adding a second shape for one payload and a drift duty to go with
    it. That was this document's first design, and a review found it: the mechanism protected
    a boundary the renderer had already crossed.
*   **Why not make the page exact?** Because exactness is the filed artifact's job and the
    page would then print `1234.56500000000000000000` to a person. The two artifacts have
    different jobs: one is filed, one is read. Making the page exact also means a
    decimal rounder that is right about scale, sign, `-0` and exponent input. The repo has
    one partial precedent — `sellPnlCsv.ts:61-65`, which handles the exponent case and
    documents the `1e21` `toFixed` hazard — but it is a string-shaper for values already
    through `Number()`, not a decimal rounder, so the cost is real rather than unlimited.
*   **The double is a smaller hazard than the first draft claimed, and the correction is
    recorded with its limit.** The first draft said a printed figure could differ from the
    CSV "by a cent" because of `Number()`. At the magnitudes these portfolios reach, that
    needs the input to carry more significant digits than a double holds (~15-17), and the
    engine's products do carry them: `tax_engine.rs:414` is
    `(tx_qty * tx_price + tx_fee) / tx_qty` and the FX multiplications at `:418`, `:422` and
    `:433` have no scale control, so a value like `1234.56499999999999999999` is possible and
    rounds to `1,234.57` through a double where exact decimal half-up gives `1,234.56`. The
    probability in practice is negligible and the divergence nobody has measured is
    quantisation, which is why Goal 3 declares that instead of denying it. This claim is
    scoped to inputs of 15 significant digits or fewer; beyond that the residual is the
    truncation above, and it is not claimed to be zero.
*   **What is still required of the payload:** the statement reads `strategy`,
    `threshold_days` and `currency` from the payload rather than from client state, since
    the server echoes all three (`analytics.rs:98-100`) and `normalizeTaxSummary`
    (`decimal.ts:63`) currently keeps only `currency`. That is two added fields on the
    hand-written type and two lines in the normalizer — additive, and it moves no figure.

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

It does five things, in this order:

1.  `@page { size: A4; margin: 14mm }`. Letter users change it in the print dialog; the
    choice is stated rather than hidden. `@page` has no scope, so it applies to a print
    started from any tab — see the scoping note below.
2.  **Hide the shell, not a list of things inside it.** Two rules, in this order:

    ```css
    .app-container > *:not(.main-content) { display: none }
    .main-content > *:not([data-print="statement"]) { display: none }
    ```

    The first rule removes the sidebar (`<aside className="sidebar">`, `App.tsx:379`) and
    anything else that is a direct child of `.app-container` — which includes the
    auth-failed alert at `App.tsx:359-376`, a `position: fixed` full-width red banner that an
    enumerated hide list would print on every page. The second leaves only the statement
    inside the content column, so the header row (`<div className="header-row">`,
    `App.tsx:458`) and any future chrome added to that column disappear without anyone
    remembering to add them to a list.

    An earlier draft enumerated `.sidebar`, `.header-row`, `[data-no-print]`. That list was
    already wrong when written — it missed the auth alert — and an allowlist of what to hide
    drifts every time someone adds chrome, in the same direction, silently. The inclusive
    form cannot miss. `[data-no-print]` is still honoured, for the Print control and anything
    else that opts out from inside the printed subtree.

    **The selectors are named here on purpose.** The parent design's file list cited
    `header.app-header` at `App.tsx:385-396`; that element does not exist. A gate that greps
    for a selector nobody wrote is worse than no gate.
3.  Collapse the layout: `.app-container` is a `260px 1fr` grid (`index.css:93-98`); in
    print it becomes one column, or the page prints with a 260 px gutter of nothing.
4.  Invert: `--bg-dark`, `--bg-card`, `--border-color`, `--text-primary`,
    `--text-secondary`, `--text-muted` to print values; `.glass-card` loses
    `backdrop-filter`, `box-shadow` and its translucent background. Sign is carried by the
    number, never by colour: `formatCurrency` already renders negatives as `-$1,234.56`,
    so a greyscale printer loses nothing.
5.  Break rules: `tr { break-inside: avoid }`, and the headline block never splits.

**Scoping the inversion.** The rules above live under `@media print` and are not scoped to the
statement tab, so Ctrl-P from the dashboard after this ships produces a light-on-dark page
with the sidebar hidden and the dashboard printed — better than today's white-on-white, but not
a designed output. Some backgrounds are not variables and will not flip: `.sidebar` is
`rgba(10, 15, 28, 0.95)` (`index.css:101`), `.badge-crypto` (`:334`),
`.data-table tr:hover td` (`:252`). The decision recorded here is that **printing from another
tab is out of scope and stays that way**: the statement is the tab you print from, and the
statement tab is the only one the smoke exercises. If that ever becomes wrong, the fix is a
body attribute set by the statement tab and every print rule scoped under it — one attribute,
not a rewrite.

### Verification, and what it cannot cover

Three layers, weakest first, with the gap named at each:

1.  **Vitest component test** (`frontend/src/test/components/PortfolioStatement.test.tsx`).
    jsdom does not apply print CSS, so this proves structure and formatting only. The
    formatting assertion is a **golden**, not a comparison: a fixture payload carrying a
    half-cent value (`1234.565`) and an expected literal rendered string in the test. An
    assertion of the form "the printed figure equals the payload value rendered through
    `formatters.ts`" cannot fail for any component that calls `formatCurrency`, and rounding
    the payload with a double to build the expected value would re-run the renderer's own
    path — the same unfailable-test failure this document uses to reject the grep gate. The
    structural assertions: the statement root carries `data-print="statement"`; the printed
    subtree contains no `button`, `input`, `select` or `svg`; all three printed headline
    figures (cumulative realized, unrealized, portfolio value) appear; the rounding line is
    present; and the run-context line carries the payload's echoed `strategy` and
    `threshold_days` rather than a client-side copy, with `threshold_days` absent for a
    non-HYBRID fixture. The mock at `handlers.ts:121` is branched, not duplicated, and its
    money fields are changed to decimal strings to match what the server actually sends; the
    existing handler ordering comment at `handlers.ts:130-131` still applies. **No assertion
    in this file may compare two normalizers** — an earlier draft asked for that and it cannot
    fail, since both read the same object and both end up doubles at render.
2.  **Playwright smoke** (`frontend/e2e/statement.smoke.mjs`, following
    `sell-pnl.smoke.mjs`). `page.emulateMedia({ media: 'print' })`, then assert the statement
    is **visible** (non-zero box, dark computed text color on a light background) and that the
    shell is gone — `.sidebar` and the auth alert compute `display: none`. Asserting the
    statement is visible is the primary check; asserting an enumerated list of hidden selectors
    is not, because the hide rule is now inclusive and a list would only re-state a rule that
    already cannot miss. The text-color assertion is what actually catches the dark-theme
    failure, and it is a computed-style check, not a screenshot. Also assert no horizontal
    overflow, and assert `page.pdf()` is non-empty and one page for the two-row fixture —
    **for the fixture only**, with no claim about larger portfolios (Decided item 2). **Local step, not a gate**: `e2e/run.sh` hardcodes
    `node e2e/sell-pnl.smoke.mjs` and must be parameterised first, and `ci.yml` has no
    browser job. Both are issue #45.

    **Why this stays local, and what that claim is actually resting on.** "Ships unverified by
    design" was inherited from the parent design, and it is weaker than it looks: CI runs
    `ubuntu-latest`, where `npx playwright install chromium` is the ordinary one-liner and
    `emulateMedia`/`page.pdf()` need no display server. The remedy this document first named —
    `nix-shell -p chromium -p liberation_ttf` — is the author's workstation, not CI, and
    saying "no browser in CI" without checking which CI was meant is exactly the kind of
    inherited constraint this review exists to catch. It stays local anyway, for two reasons
    that survive the correction: `e2e/run.sh:3-16` is not browser-only — it compiles and
    starts the Rust API against a throwaway SQLite file and starts Vite — so the job costs a
    full cargo build and needs seeded data; and a job marked `continue-on-error` is
    decoration unless someone reads it, which converts "unverified" into "verified and
    therefore fine" by another door. The honest options are **local step** (chosen) or **a
    real blocking browser job that also runs the existing sell-P&L smoke** — which is issue
    #45's scope, not this slice's. The named assertions above are handed to #45 so they land
    with the job that can execute them.
3.  **A human with a print preview.** `docs/auth-testing.md` §5 is already the manual
    browser walkthrough for what CI cannot check; the statement's print preview joins that
    list.

### Sequencing

One slice. There is no useful smaller cut: the page without the print stylesheet is a
dashboard with a different heading, and the print stylesheet without the page has nothing to
style.

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
*   **Proposed Changes:** the five steps above, with the selectors named exactly as in
    Palette inversion.

#### 3. `frontend/src/utils/decimal.ts` (Modify) and `frontend/src/types.ts` (Modify)

*   **Type / Scope:** surface the two run-context fields the payload already sends.
*   **Proposed Changes:** `normalizeTaxSummary` keeps `strategy` and `threshold_days`
    (`decimal.ts:63` currently keeps `currency` only) and `TaxSummary` in `types.ts` gains
    both. Additive; no existing figure moves. There is **no** second normalizer and no
    string-typed mirror type — see Money: a display artifact, declared, for why the first
    draft of this document was wrong to add them.

#### 4. `frontend/src/App.tsx` (Modify)

*   **Type / Scope:** the fifth nav item and the tab branch.
*   **Proposed Changes:** renders the statement from the `taxSummary` state
    `fetchPortfolioData` already sets, guarded the way the sibling tab guards
    (`activeTab === 'portfolios' && taxSummary`, `App.tsx:483`) so a refresh in flight cannot
    print zeros. Base currency for the header comes from the portfolio object, **not** from
    the payload: the empty-portfolio branch at
    `backend_rust/src/api_routes/analytics.rs:45-51` returns no `currency` key, and
    `normalizeTaxSummary` then defaults it to `USD` — so an empty EUR portfolio would print
    "USD".

#### 5. `frontend/src/test/components/PortfolioStatement.test.tsx` (Create) and `frontend/src/test/mocks/handlers.ts` (Modify)

*   **Type / Scope:** structural tests and the mock branch.
*   **Proposed Changes:** the assertions listed under Verification layer 1. The mock at
    `handlers.ts:122` must also be corrected: it returns `total_realized_pnl: 0` as a JSON
    **number** while the backend always sends strings (`analytics.rs:95-97`,
    `db_types.rs:31`), so the fixture has to carry decimal strings and the two echoed
    context fields, or the statement is tested against a shape the server never emits.

#### 6. `frontend/e2e/statement.smoke.mjs` (Create), `frontend/e2e/run.sh` (Modify), `frontend/package.json` (Modify)

*   **Type / Scope:** the browser smoke, and the parameterisation it needs.
*   **Proposed Changes:** as in Verification layer 2. `run.sh` currently hardcodes
    `node e2e/sell-pnl.smoke.mjs`; it gains a target argument defaulting to the existing
    smoke, and `package.json` gains `e2e:statement`. **Not wired into CI** — see Alternative
    B.

#### 7. Deleted — no print-CSS grep gate

Carried from the parent design. A grep asserting a stylesheet mentions some selectors goes
green while the page still prints badly. The Playwright smoke is the honest check; a grep
that can pass while the feature is broken is a gate that teaches people to ignore gates.

## Risks

| Risk | Evidence | Mitigation |
| :-- | :-- | :-- |
| **The print path ships unverified.** No CI browser job; the smoke is a local step needing a chromium binary. The reason is a workstation fact, not a CI law — see Verification layer 2. | `ci.yml` has no browser job; issue #45 open | Selectors and smoke assertions are named in this document and handed to #45, so the check exists and is specific the moment that job lands. The manual print preview is listed in `docs/auth-testing.md` §5. |
| **Palette override misses a hardcoded color.** | `SellPnlModal.tsx:132` uses inline `background: '#121929'` | The statement uses no hardcoded colors; the smoke asserts computed text color, which catches the failure regardless of cause. |
| **A printed figure differs from the CSV's.** | The CSV is full-precision (`reviews/2026-11-05-…:18`) and `formatCurrency` quantises to 2 dp (`formatters.ts:4`) | Goal 3: the page declares its rounding, so the difference is stated rather than discovered by an adviser |
| **A third renderer of one payload, with nothing comparing it to the second.** | `PortfolioDetail.tsx:238-292` renders the same `taxSummary.assets`; `Dashboard.tsx:40-45` renders the same headline figures | Accepted knowingly (Alternative D). The statement is a leaf with no interactivity and takes currency from the payload, not from `portfolio?.currency \|\| 'USD'` as `PortfolioDetail.tsx:261` does. If the two tables ever need to agree, the check is a comparison test between them, and it is not written here. |
| **A4 on Letter hardware.** | `@page { size: A4 }` | Stated in the dialog, not hidden; the owner's portfolios are EUR-denominated. |
| **Statement read as a quarterly figure.** | The undated endpoint has no window | Goal 1's label requirement: "cumulative since inception" is on the page, not implied. |
| **Empty or failing portfolio prints zeros.** | `analytics.rs:45-51` returns zeros with no `currency` | Header currency from the portfolio object; the empty case prints an explicit "no holdings" line rather than three zeros. |

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
    headless browser on the server. The app runs in a 256 Mi single-replica pod.
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
    eventually by everything. This is the only variant that would actually work. **Rejected
    because exactness is the wrong property for this artifact:** a page read by a person should
    not print `1234.56500000000000000000`, and the filed artifact — the tax report CSV, once
    it ships — is the one meant to be exact. It also means hand-writing a decimal rounder (scale, sign, `-0`, exponent input)
    that the repo does not have today, and if it lands inside `formatCurrency` it changes what
    every existing screen prints for >2-dp values — the one thing this slice promised not to
    do.
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
