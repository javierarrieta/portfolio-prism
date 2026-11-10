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
printing a dark-themed app honestly, and about the printed numbers agreeing with the CSV to
the cent.

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
*   **The frontend throws away the decimals before anything renders them.** The backend
    sends money as decimal strings (`backend_rust/src/db_types.rs:31`,
    `serializer.serialize_str(&d.to_string())`), and `normalizeTaxSummary` in
    `frontend/src/utils/decimal.ts` then runs every money field through `num()`, which is
    `Number(v)`. So `taxSummary.total_realized_pnl` is a JS double by the time any component
    sees it. A statement printed from that value can disagree with the tax report CSV by a
    cent — and the CSV is the file that goes to the tax authority. This is the one defect in
    this slice that is a correctness problem rather than a formatting problem.
*   **There is no router.** `App.tsx:36` is `useState(DEFAULT_TAB)` and the nav is four
    `<li className="nav-item">` elements (`App.tsx:385-396`). There is no URL to open a
    statement at, so there is no link to re-open a printed page later and no way to bookmark
    it. Adding a router for one page is not worth it and is a non-goal.
*   **Nothing can prove it prints.** `.github/workflows/ci.yml` has no browser job. Issue
    #45 (*Run the sell P&L Playwright smoke in CI*) is open. This slice therefore **ships
    unverified by design** — see Risks. That was an explicit owner decision, not an
    oversight, and it is the reason this document names its selectors and its smoke
    assertions here rather than leaving them to implementation time.

## Goals

*   **Goal 1 — the two headline figures, honestly labelled.** Cumulative realized P&L and
    unrealized P&L for the selected portfolio, with realized labelled as **cumulative since
    inception**. The undated `tax-summary` endpoint has no window, so a statement cannot
    show a quarterly realized figure; an unlabelled number next to a quarterly tax form is
    how a wrong filing starts.
*   **Goal 2 — the printed page is a document.** No sidebar, no navigation, no buttons, no
    inputs, no charts, no error banner. One column. `@page { size: A4; margin: 14mm }`.
*   **Goal 3 — the printed numbers are the payload's numbers.** Every money value on the
    statement is the payload's decimal string rendered through
    `frontend/src/utils/formatters.ts`. The statement performs no arithmetic: it sums
    nothing, differences nothing, and rounds nothing that the payload did not already round.
    For the same portfolio at the same moment, the statement's cumulative realized figure
    and the tax report's cumulative realized column are the same decimal, and that equality
    is a test.
*   **Goal 4 — the page states its own provenance.** Portfolio name, base currency, the
    strategy and `threshold_days` the request was made with — labelled as the strategy in
    use, because the payload does not confirm them (Decided item 3) — the generation date,
    and the standing caveat that splits are as recorded at that moment.
*   **Goal 5 — no backend change.** No new route, no new field, no new figure on any
    existing endpoint, no OpenAPI regeneration. If this slice needs the backend, it has
    drifted out of scope and should stop.
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

### Money fidelity: a second normalizer, not a patched one

`normalizeTaxSummary` stays exactly as it is. Every existing screen keeps its doubles, so
nothing on the dashboard or the holdings table moves.

A sibling, `normalizeTaxSummaryStrings`, keeps every money field as the payload's own
string and is used only by the statement. Its type is a string-typed mirror of
`AssetTaxSummary`/`TaxSummary` in `frontend/src/types.ts`.

*   **Why not just fix `normalizeTaxSummary`?** Because that changes what every existing
    component holds, and this slice's contract is that it moves no existing figure. It is
    also a much larger test surface than a statement deserves.
*   **Why not let the statement use doubles?** Goal 3. A statement that prints `1,234.56`
    while the CSV says `1234.565` is the exact failure mode this whole design exists to
    remove, and it is invisible when it happens.
*   **The cost, stated:** two shapes for one payload, which can drift. The guard is a test
    that renders both from one fixture and asserts they agree to the cent on every field.
    That test is the reason this is a decision rather than a hazard.

### Palette inversion

The palette is variable-driven (`index.css:3-31`), and components read it through the
variables — including inline styles such as
`style={{ color: 'var(--text-secondary)' }}` in `SellPnlModal.tsx:206`. That is what makes
this cheap: `print.css` overrides the variables on `:root` inside `@media print` and most
of the app flips at once, inline styles included.

`frontend/src/styles/print.css` (new, imported from `main.tsx` after `index.css`) does five
things, in this order:

1.  `@page { size: A4; margin: 14mm }`. Letter users change it in the print dialog; the
    choice is stated rather than hidden.
2.  Hide chrome: `.sidebar`, `.header-row`, `[data-no-print]`. **The selectors are named
    here on purpose.** The parent design's file list cited `header.app-header` at
    `App.tsx:385-396`; that element does not exist — the header is
    `<div className="header-row">` at `App.tsx:458`, and the nav is `.nav-menu`/`.nav-item`
    inside `<aside className="sidebar">` at `App.tsx:379`. A gate that greps for a selector
    nobody wrote is worse than no gate.
3.  Collapse the layout: `.app-container` is a `260px 1fr` grid (`index.css:93-98`); in
    print it becomes one column, or the page prints with a 260 mm gutter of nothing.
4.  Invert: `--bg-dark`, `--bg-card`, `--border-color`, `--text-primary`,
    `--text-secondary`, `--text-muted` to print values; `.glass-card` loses
    `backdrop-filter`, `box-shadow` and its translucent background. Sign is carried by the
    number, never by colour: `formatCurrency` already renders negatives as `-$1,234.56`,
    so a greyscale printer loses nothing.
5.  Break rules: `tr { break-inside: avoid }`, and the headline block never splits.

The statement component itself uses **no hardcoded colors**. Inline `#121929`-style
backgrounds (`SellPnlModal.tsx:132`) are exactly what a variable override cannot reach, and
the statement must not repeat that mistake.

### Verification, and what it cannot cover

Three layers, weakest first, with the gap named at each:

1.  **Vitest component test** (`frontend/src/test/components/PortfolioStatement.test.tsx`).
    jsdom does not apply print CSS, so this proves structure only: the statement root
    carries `data-print="statement"`, the printed subtree contains no `button`, `input`,
    `select` or `svg`, the two headline figures equal the payload strings rendered through
    `formatters.ts`, and the string normalizer and the numeric normalizer agree to the cent
    on one fixture. `handlers.ts:121` already registers
    `/api/portfolios/:id/tax-summary`; branching it is enough, and the existing handler
    ordering comment at `handlers.ts:130-131` still applies.
2.  **Playwright smoke** (`frontend/e2e/statement.smoke.mjs`, following
    `sell-pnl.smoke.mjs`). `page.emulateMedia({ media: 'print' })`, then assert computed
    `display: none` on the named chrome selectors, assert the statement's computed text
    color is dark against a light background — that is the assertion that actually catches
    the dark-theme failure, and it is a computed-style check, not a screenshot — assert no
    horizontal overflow, and assert `page.pdf()` is non-empty and one page for the two-row
    fixture — **for the fixture only**, with no claim about larger portfolios (Decided
    item 2). **Local step, not a gate**: `e2e/run.sh` hardcodes
    `node e2e/sell-pnl.smoke.mjs` and must be parameterised first, and `ci.yml` has no
    browser job. Both are issue #45.
3.  **A human with a print preview.** `docs/auth-testing.md` §5 is already the manual
    browser walkthrough for what CI cannot check; the statement's print preview joins that
    list.

### Sequencing

One slice. There is no useful smaller cut: the page without the print stylesheet is a
dashboard with a different heading, and the print stylesheet without the page has nothing to
style.

| Slice | Ships | Reaches network in tests? | Moves an existing figure? |
| :-- | :-- | :-- | :-- |
| 1 | Statement tab, print stylesheet, string normalizer, component test, local smoke | no | no |

## Target Files

#### 1. `frontend/src/components/PortfolioStatement.tsx` (Create)

*   **Type / Scope:** the printable view.
*   **Proposed Changes:** renders the string-typed summary: headline block (cumulative
    realized since inception, unrealized as of today, portfolio value) and one row per
    holding (Decided item 1). No asset-subset filter — the only arithmetic that would make a
    filtered headline honest is the arithmetic renderers are forbidden from doing. Takes no
    date parameter. Derives no totals. No `button`, `input` or hardcoded color inside the
    printed subtree; the Print control sits outside it with `data-no-print`.

#### 2. `frontend/src/styles/print.css` (Create) and `frontend/src/main.tsx` (Modify)

*   **Type / Scope:** the print stylesheet, imported after `index.css` so it wins the
    cascade at equal specificity.
*   **Proposed Changes:** the five steps above, with the selectors named exactly as in
    Palette inversion.

#### 3. `frontend/src/utils/decimal.ts` (Modify) and `frontend/src/types.ts` (Modify)

*   **Type / Scope:** `normalizeTaxSummaryStrings` plus its string-typed mirror types.
*   **Proposed Changes:** additive. `normalizeTaxSummary` is untouched.

#### 4. `frontend/src/App.tsx` (Modify)

*   **Type / Scope:** the fifth nav item, the tab branch, and the raw payload the statement
    renders.
*   **Proposed Changes:** `fetchPortfolioData` keeps the string-typed summary alongside the
    normalized one it already sets. Base currency for the header comes from the portfolio
    object, **not** from the payload: the empty-portfolio branch at
    `backend_rust/src/api_routes/analytics.rs:45-51` returns no `currency` key, and
    `normalizeTaxSummary` then defaults it to `USD` — so an empty EUR portfolio would print
    "USD".

#### 5. `frontend/src/test/components/PortfolioStatement.test.tsx` (Create) and `frontend/src/test/mocks/handlers.ts` (Modify)

*   **Type / Scope:** structural tests and the mock branch.
*   **Proposed Changes:** the four assertions listed above, including the two-normalizers
    agree-to-the-cent test, which is what makes two shapes for one payload a decision rather
    than a latent bug.

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
| **The print path ships unverified.** No CI browser job; the smoke is a local step needing a chromium binary. | `ci.yml` has no browser job; issue #45 open | Selectors and smoke assertions are named in this document, so the check exists and is specific the moment #45 lands. The manual print preview is listed in `docs/auth-testing.md`. |
| **Palette override misses a hardcoded color.** | `SellPnlModal.tsx:132` uses inline `background: '#121929'` | The statement uses no hardcoded colors; the smoke asserts computed text color, which catches the failure regardless of cause. |
| **Two normalizers drift.** | `decimal.ts` is hand-written per field | The agree-to-the-cent test on one fixture. |
| **A4 on Letter hardware.** | `@page { size: A4 }` | Stated in the dialog, not hidden; the owner's portfolios are EUR-denominated. |
| **Statement read as a quarterly figure.** | The undated endpoint has no window | Goal 1's label requirement: "cumulative since inception" is on the page, not implied. |
| **The strategy line asserts more than the payload confirms.** | The undated payload echoes neither `strategy` nor `threshold_days` | Labelled "strategy in use", never "confirmed"; Decided item 3 states the condition that would retire it |
| **Empty or failing portfolio prints zeros.** | `analytics.rs:45-51` returns zeros with no `currency` | Header currency from the portfolio object; the empty case prints an explicit "no holdings" line rather than three zeros. |

## Decided during drafting

Three items were open when this document was first written. They are settled here, and the
reasons are recorded because each one trades something away.

1.  **The statement lists holdings.** The two headline figures are the point, and a 40-row
    table is a second page and a second set of numbers to reconcile against the CSV. It
    lists them anyway: a statement without holdings is a receipt, and the owner's use case
    is handing something to a person who will ask "which positions?". The reconciliation
    cost is real and is paid by Goal 3 — every printed row is a payload value, so the rows
    cannot drift from the CSV; what they can do is run to a second page, which is a layout
    fact, not a correctness one.
2.  **The smoke asserts one page for its own fixture and says nothing about larger
    portfolios.** One page for a two-row fixture is checkable. A bound that holds for 50
    holdings is a layout claim nobody has measured, and an assertion that fails on the
    owner's real portfolio but passes in CI is an assertion that teaches people to ignore
    the smoke. The page-break rules still ship (`tr { break-inside: avoid }`); they are
    simply not claimed to produce a page count.
3.  **The strategy line prints the value the frontend sent, labelled as the strategy in
    use.** The undated payload echoes neither `strategy` nor `threshold_days`, so the
    server's value is unavailable without a backend change this document refuses to make.
    The sent value is honest as long as it is labelled as what it is; it stops being honest
    the moment it is presented as confirmed by the server. If the tax report's payload ends
    up echoing both, the statement should read them from the payload and this decision
    should be revisited rather than left to rot.

## Alternatives Considered

### Alternative A: server-generated PDF

*   **Pros**: exact pagination, no browser print dialog, a real artifact to hash and file.
*   **Cons**: a new Rust dependency with a font-file requirement (`printpdf`, `genpdf`) or a
    headless browser on the server. The app runs in a 256 Mi single-replica pod.
*   **Reason for rejection**: the statement is read or emailed, not filed or signed. The CSV
    is the filed artifact and it is already exact. Browser print-to-PDF reuses the existing
    formatters, so the numbers look like the rest of the app for free.

### Alternative B: a print-CSS grep gate in CI

*   **Pros**: something runs on every push; cheap.
*   **Cons**: it asserts that a stylesheet mentions selectors, not that the page prints. It
    passes on a page that prints white-on-white.
*   **Reason for rejection**: a gate that can pass while the feature is broken is worse than
    an honest local step, because it converts "unverified" into "verified and therefore
    fine". The Playwright smoke is the gate; it is waiting on issue #45, which is the same
    blocker as the existing sell-P&L smoke.

### Alternative C: fix `normalizeTaxSummary` to keep strings

*   **Pros**: one shape for the payload; the doubles bug is gone everywhere, not just here.
*   **Cons**: every component that does arithmetic on those fields changes, which is most of
    them, and every one of those numbers is a figure this slice promised not to move.
*   **Reason for rejection**: correct but out of scope. Recorded here so the next person
    does not rediscover it. If it is ever done, it should be its own change with its own
    figure note.

## Verification

*   `npm test`, `npm run lint`, `npm run typecheck` from `frontend/`.
*   `npm run e2e:statement` from `frontend/` — **local step, not a gate**. Needs a chromium
    binary and fonts; on NixOS,
    `nix-shell -p chromium -p liberation_ttf --run './e2e/run.sh statement'`.
*   Manual: open the statement, print preview, confirm no sidebar, no nav, no print button,
    dark text on white, and that the two headline figures match the dashboard to the cent.
    Added to the manual browser walkthrough in `docs/auth-testing.md` §5.
*   Cross-check with the tax report once it exists: same portfolio, same moment, statement's
    cumulative realized equals the CSV's cumulative realized column. Until the tax report
    ships, the equality is asserted against the payload in the component test.
