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

That is the appeal of this slice and also its limit. The hard parts are printing a dark-themed
app legibly, and making a page whose figures are rounded say so rather than pretend to be the
exact file next door.

## The Problem

*   **There is no printable view.** `grep -r "@media print" frontend/src` returns nothing. The
    owner's only option today is Ctrl-P on the dashboard, which prints the sidebar, the navigation,
    the error banner and a Recharts SVG.
*   **The app is dark glassmorphism, and dark backgrounds are not printed by default.**
    `index.css:6-30` sets `--bg-dark: #090d16` and `--text-primary: #f8fafc`, and `.glass-card`
    uses `backdrop-filter: blur(16px)`. Browsers omit background colors unless the user turns on
    background graphics, so a naive print is near-white text on white paper — invisible, not merely
    ugly. The print stylesheet's real job is **palette inversion**; hiding the nav is the easy
    part.
*   **The printed page must say what it rounded.** The backend sends money as full-precision
    decimal strings (`db_types.rs:31`) and the engine multiplies by the FX rate with no scale
    control (`tax_engine.rs`, the FX multiplications in `replay`), so a realized figure carries
    more decimals than a page should show. The frontend rounds at display, to whatever `Intl`
    decides per currency code. A printed figure and the tax report CSV's figure for the same
    holding therefore show **different digits** — by design, one being rounded and the other not —
    and are the same number once both are rounded identically. A page printing `1,234.57` beside a
    CSV cell reading `1234.56500000000000000000`, while claiming to be the same number, is the
    defect. The fix is to say the rounding out loud on the page.
*   **There is no router, so the statement is not linkable.** `App.tsx:36` is
    `useState(DEFAULT_TAB)`; the nav is four `<li className="nav-item">` elements
    (`App.tsx:386-398`). A printed page cannot be re-opened by URL or bookmarked, and this slice
    does not solve that. Rejected: Alternative G.
*   **Nothing can prove the print path works.** `.github/workflows/ci.yml` has no browser job and
    issue #45 (*Run the sell P&L Playwright smoke in CI*) is open, so **the print path ships
    unverified**: the component test verifies structure and rounding, but no automated check
    renders the page under print media. The reasons that stays a local step are in Verification
    layer 2, which also records that CI *could* run it — a choice about CI cost, not an
    impossibility. Selectors and smoke assertions are named here rather than left to
    implementation time, so the check is specific the moment #45 lands.
*   **The person the statement is for cannot log in.** Every `/api` route takes `AuthedUser` plus a
    `require*` call, and `apiFetch.ts:22-26` sends an unauthenticated caller to `/auth/login`. An
    adviser has no account. The artifact necessarily leaves the app: the owner prints, then
    forwards. So the page must stand on its own once detached, which is what Goal 4's provenance
    block and Goal 3's declared rounding are for. Rejected: Alternative F.

## Goals

*   **Goal 1 — the P&L figures, labelled.** Cumulative realized P&L and unrealized P&L for the
    selected portfolio, with realized labelled **cumulative since inception**; portfolio value
    prints alongside and needs no such label. The undated `tax-summary` endpoint has no window, so
    a statement cannot show a quarterly realized figure, and an unlabelled number next to a
    quarterly tax form is how a wrong filing starts.
*   **Goal 2 — the printed page is a document.** One column. No sidebar, navigation, buttons,
    inputs, `select` elements, charts or SVG, error banner, or horizontal overflow at A4 width.
    `@page { size: A4; margin: 14mm }`, with the caveat from Palette inversion that `@page` cannot
    be scoped and so also changes the paper size of prints started from other tabs.
*   **Goal 3 — the printed numbers are the payload's numbers, rounded once and declared.** The
    statement is a **display artifact**. Every money value is a payload value rendered through
    `frontend/src/utils/formatters.ts` and rounded at display, nowhere else. The rule is
    "whatever `formatters.ts` renders", not "to the cent": `EUR` renders 2 dp, `JPY` renders
    **0 dp** (`1234.565` → `¥1,235`), `BTC`/`ETH` up to 8 (`formatters.ts:10-11`), and any other
    crypto code falls to the `catch` at `formatters.ts:19-22` at exactly 2 dp. Every other screen
    follows the same rule. The statement performs no arithmetic: it sums nothing, differences
    nothing, never re-derives a total. The engine's output is full precision, so the page prints a
    visible line saying its figures are rounded from that precision. When the tax report ships, its
    cumulative realized column should equal the statement's figure rounded the same way
    `formatters.ts` rounds it; that cross-check is conditional on the tax report existing. The
    tests that ship with this slice are Verification layer 1's golden-value test and layer 2's
    smoke.
*   **Goal 4 — the page states its own provenance.** Portfolio name, base currency, the strategy
    and `threshold_days` as the server echoed them (`analytics.rs`, the tail of the tax-summary
    response), the printed date, the data-fetched timestamp, and the splits caveat — each with its
    hook and literal copy in *On-page copy*. `threshold_days` prints only for `HYBRID`, because it
    is unused by `FIFO` and `LIFO` (`tax_engine.rs:645-649`). The empty-portfolio branch
    (`analytics.rs:45-51`) echoes none of the three fields, so the run-context line is omitted
    there rather than rendered from `str()`/`num()` defaults; a line reading
    "realized · (threshold 0d)" would be a fabricated run context. See *What "splits" and the
    strategy line mean*, including the narrower claim this slice can actually make about the
    echoed strategy.
*   **Goal 5 — no backend change in this slice.** No new route, no new field, no new figure on any
    existing endpoint, no OpenAPI regeneration. This is a **sequencing** boundary, not a standing
    principle: the parent design will rewrite this handler's response and add `from`/`to`. The
    handler currently returns `Json<serde_json::Value>`, which is why the committed
    `docs/openapi/openapi.json` gives this response an empty schema (`:326-334`). If
    implementation requires a backend change, stop and re-scope. The boundary's cost: the run
    context printed here is a subset of what the parent design will carry anyway. It is kept
    because any response-shape change lands in a file CI diffs byte-for-byte, so it is not a quiet
    two-field change — and because the statement is the one slice of this programme not blocked on
    the pricing layer.
*   **Goal 6 — the palette inversion does not depend on a browser setting.** It must not rely on
    "background graphics" being ticked: text becomes dark because the stylesheet says so, not
    because the browser kept a background. The second half of the requirement is the case the
    setting *is* on — then the background overrides must also be present, or that reader gets dark
    text on the dark app background. Palette inversion step 4 says which variables carry which
    half.
*   **Goal 7 — the empty portfolio says so.** `analytics.rs:45-51` returns three zeros and no
    holdings. Printing three zeros under a "Portfolio Statement" heading misrepresents the state,
    so that branch prints the heading, the printed date and the empty note, and nothing else. See
    *Empty portfolio* for the branch and the negative list.

## Non-Goals

*   **No date parameter, and no past-period statement.** Decided in the parent design.
*   **No server-side PDF, and no PDF dependency.** See Alternative A.
*   **No print-CSS gate in CI.** See Alternative B and Risks.
*   **No router, no deep link, no shareable statement URL.**
*   **No per-asset price-date provenance on the statement.** The undated endpoint returns no
    `price_date_used`, so the page states only what it can know: the portfolio, the strategy, the
    printed date and the data-fetched timestamp. Per-asset pricing provenance belongs to the tax
    report, which does not exist yet.
*   **No cross-portfolio statement.** One portfolio, as in the tax report.
*   **No print theme picker, no "dark print" mode.**

## Technical Plan

### Surface: a fifth tab, not a route

`statement` joins `dashboard`, `portfolios`, `analytics` and `ledger` in the existing nav
(`App.tsx:386-398`). It renders from the `taxSummary` state that `fetchPortfolioData`
already populates (`App.tsx:111`), so opening the statement costs no request and cannot fail
in a way the dashboard does not already fail.

The Print control is a button **outside** the printed subtree, calling `window.print()`, and
positioned as a sibling of the statement root so the hide rules remove it — see *Palette
inversion*, rule 2.

### On-page copy, written out

Six strings are requirements rather than implementation detail — Goals 1, 3, 4 and 7 each assert
one — so each is written here with a stable hook. An assertion that "the rounding line is present"
is not an assertion until the string or the hook is fixed. Copy is English, matching the rest of
the app's UI, and assertions match `textContent`, so these are literal text and not markup: write
`P&L`, not `P&amp;L`.

| Hook | Copy | Printed on the empty branch? |
| :-- | :-- | :-- |
| `data-print="label-realized"` | `Realized P&L — cumulative since inception` | no |
| `data-print="generated-at"` | `Printed {printDate}` — `printDate` is `YYYY-MM-DD`, from the client clock at render | yes |
| `data-print="data-at"` | `Data fetched {fetchTime}` — ISO 8601 to the minute, from the timestamp captured in `fetchPortfolioData` | no |
| `data-print="rounding-note"` | `Figures are rounded for display from full-precision calculations.` | no |
| `data-print="splits-note"` | `Split-adjusted history is as recorded when this page was loaded; late-recorded splits can change past figures.` | no |
| `data-print="empty-note"` | `No holdings in this portfolio on {printDate}.` | yes, alone |
| `data-print="run-context"` | `{portfolio name} · {base currency} · {strategy}`, and when strategy is `HYBRID` a trailing ` (threshold {threshold_days}d)` — the leading space is part of the appended segment, not of the base string | no |

Three rules that fall out of the table:

*   **The rounding note is unconditional.** It prints even for a currency whose `Intl` rendering
    is already exact for the values in hand, because the engine's precision is not the currency's
    precision. Computing when it would be redundant is not worth the branch.
*   **Two dates, two clocks, and the page says which is which.** The statement renders cached
    state and issues no request on open, so a page printed the morning after a session opened
    yesterday carries today's date over yesterday's prices. `generated-at` is the client render
    date; `data-at` is captured in `fetchPortfolioData` beside `setTaxSummary` and is the only
    claim on the page about when the numbers were true. The splits note therefore says "when this
    page was loaded" and not a date: the split rows were as recorded when the **request** ran,
    which is a server fact a client clock cannot assert. This is the largest provenance hole the
    rounding declaration does not cover, and it is why `data-at` exists even though the payload
    carries no date field of its own.
*   **The copy never names an artifact that does not exist.** The tax report and its CSV are
    specified in a sibling design and are not built; a printed page an adviser may ask about
    cannot reference a file the owner cannot produce. When the tax report ships, the rounding note
    gains the CSV sentence in that change, not this one.

### Empty portfolio

`analytics.rs:45-51` returns `assets: []`, three zero strings and **no** `currency`, `strategy`
or `threshold_days`. Goal 7 is that branch: the component checks `taxSummary.assets.length === 0`
and renders the portfolio heading, `generated-at` and `empty-note` — and nothing else. No headline
figures, no `data-at`, no run-context, no splits note, no rounding note, no holdings table: a page
reading "No holdings in this portfolio" followed by "Split-adjusted history is as recorded…" is
nonsense. The existing holdings table already makes the same branch for the same reason
(`PortfolioDetail.tsx:238`), so this is a repo convention. Verification layer 1 asserts both the
positive and the negative list.

### Money: a display artifact, declared

The statement renders payload numbers through `formatters.ts`, which rounds at display to
whatever `Intl` decides for that currency code — the rule stated normatively in Goal 3 and not
restated here. It does not attempt to be exact, and it says so on the page.

*   **Where precision is actually lost.** Not in the formatter, which is where this document first
    put it. `normalizeTaxSummary` (`decimal.ts:65-67`) runs every money field through `num()`,
    which is `Number(v)`, and `TaxSummary` types them as `number` (`types.ts:89-95`). The
    statement renders from that state, so **the full-precision string never reaches the
    component at all** — it is gone before any statement code runs. `formatCurrency` then calls
    `toNumber` again (`formatters.ts:4`), which is a second conversion of a value that is already
    a double. Two consequences: Goal 3's "the payload carries full precision" describes the wire,
    not something the page holds, and the page's rounding declaration is a statement about the
    engine's output, not about a value the browser could recover.
*   **Why not preserve the decimal strings?** Because the conversion above happens in the shared
    normalizer that every screen uses. Preserving strings for the statement alone means a second
    payload shape and a drift duty, and it still has to survive `formatCurrency`, which coerces
    its argument to a number whatever it is given: `formatCurrency("1234.565")` and
    `formatCurrency(1234.565)` are byte-identical. See Alternative C1.
*   **Why not make the page exact?** Exactness is the filed artifact's job; a page would print
    `1234.56500000000000000000` to a person. See Alternative C2, which also covers the cost of a
    decimal rounder and the `sellPnlCsv.ts:61-65` precedent.
*   **The residual, bounded rather than disclaimed.** A double diverges from exact decimal rounding
    only once the input carries more significant digits than a double holds (~15-17), and the
    engine's products do: `tax_engine.rs` divides cost basis by quantity
    (`replay`, the `tx_qty * tx_price + tx_fee` term) and multiplies by FX rates with no scale
    control, and the review record measures `roi` at **28 fractional digits**
    (`docs/superpowers/reviews/2026-11-05-sell-pnl-breakdown.md:22`). So a value like
    `1234.56499999999999999999` is reachable: through a double it renders `1,234.57`, where exact
    decimal rounding gives `1,234.56`. This is not scoped away. The declared bound is that **a
    printed figure may differ from the exact decimal rounding of the payload by at most one unit
    of the last displayed digit**, and Verification layer 1 pins it with a golden whose payload
    exceeds 15 significant digits, so the divergence is a tested fact rather than a footnote.
    Rounding at display is the divergence that happens on every page; this one is rare and now
    bounded.
*   **What the payload supplies, field by field.** Headline figures: `total_portfolio_value`,
    `total_realized_pnl`, `total_unrealized_pnl`. Run context: `strategy`, `threshold_days`,
    `currency`, all echoed by the server (`analytics.rs`, the tail of the tax-summary response).
    Holdings rows come from `assets[]`: symbol, asset type, quantity, average cost, current
    price, market value, realized P&L, unrealized P&L, unrealized ROI — the same set
    `PortfolioDetail.tsx:238-292` already prints, minus its Actions column.
    `normalizeTaxSummary` keeps only `currency` today, so this slice adds two fields to the
    hand-written type and two lines to the normalizer. Both new fields are **optional**
    (`strategy?: string; threshold_days?: number`) precisely so the existing `TaxSummary`
    literals in `PortfolioDetail.test.tsx` and `Dashboard.test.tsx` still compile under
    `npm run typecheck`; the money fields stay `number`.
*   **Which currency formats the money, and which prints in the header.** Money formats with the
    payload's `currency`; the header prints the **portfolio object's** base currency. Not two
    competing sources: the populated branch returns `currency` **as** `portfolio.base_currency`,
    so they are the same string whenever there are holdings to format. They can differ only on the
    empty branch, which returns no `currency` key at all and would otherwise default to `USD` on a
    EUR portfolio — and that branch prints no money (Goal 7). Stated once here; Goal 4, Goal 7 and
    Target Files 4 all point at this paragraph rather than re-deriving it.

### What "splits" and the strategy line mean

Two terms on the printed page are specific to this project and are not general engineering
vocabulary.

*   **Splits.** Stock splits, auto-detected and persisted by the tax-summary handler itself —
    `get_portfolio_tax_summary` calls `sync_splits_for_asset` per asset (`analytics.rs:63-65`)
    before it computes anything. So the caveat means: the split rows in the database when the
    request ran, some of which the handler may have just added from the provider. It matters
    because a split can be recorded long after it happened, which silently rewrites history; the
    hazard is tracked as issue #50. The printed string is the `splits-note` row in *On-page copy*.
*   **`HYBRID` and `threshold_days`.** The engine supports `FIFO`, `LIFO` and `HYBRID`; the
    strategy match lives in `replay` (`tax_engine.rs`, the `match strategy` block). `HYBRID`
    consumes lots held for **`threshold_days` or fewer** first — the short-term bucket — and
    `threshold_days` is that short/long boundary in days. It is meaningless for `FIFO` and `LIFO`:
    `term_for` (`tax_engine.rs:645-649`) returns `None` for anything but `HYBRID`, which is why
    the run-context line appends it only for `HYBRID`. The on-screen precedent is
    `SellPnlModal.tsx:210`.
*   **A caveat about the echoed strategy.** The tax-summary route defaults but does not
    **normalize** strategy: `analytics.rs` takes `strategy.unwrap_or_else(|| "FIFO")` and passes
    it through, and the engine's match falls through to FIFO ordering for any unrecognized string.
    So `?strategy=average_cost` would print `average_cost` while the engine silently used FIFO.
    The sibling sell-P&L route already fixed exactly this (`analytics.rs`, the normalization block
    whose comment reads *"so the modal can never name a matching method the engine did not use"*).
    The invariant this slice can actually claim is narrower than "the server confirms the strategy":
    **the statement prints the strategy the app sent, and the app sends only `FIFO`, `LIFO` or
    `HYBRID`** (`App.tsx` builds the query from the strategy selector). Normalizing this route
    belongs to the parent design's route work. Verification layer 1 asserts that a strategy
    outside those three literals is not printed verbatim.

### Palette inversion

The palette is variable-driven (`index.css:3-31`), and components read it through the variables —
including inline styles such as `style={{ color: 'var(--text-secondary)' }}`
(`SellPnlModal.tsx:206`). That is what makes this low-effort: `print.css` overrides the variables
on `:root` inside `@media print` and most of the app flips at once, inline styles included.

`frontend/src/print.css` (new, flat beside `index.css`) is imported from `main.tsx` after
`index.css`, so it wins the cascade at equal specificity. That is source order doing the work,
not the file boundary: a `@media print` block appended to `index.css` would win just as reliably.
The separate file is for organisation; its cost is that the print palette is not next to the
palette it inverts.

**Every rule below is scoped under `body[data-print-scope="statement"]`,** which the statement
tab sets on `document.body` while it is the active tab and clears on unmount. Unscoped rules are
not acceptable here: rule 2 as `.main-content > *:not([data-print="statement"])` hides every
direct child of `.main-content` on the dashboard, analytics and ledger tabs, because nothing
there carries that attribute — Ctrl-P on three of four tabs would print a blank A4 sheet, where
today it prints the dashboard. The statement tab is the only tab this slice designs a print for,
and the scoping is what makes "the only one" true in the CSS rather than only in intent.

It does six things, in this order:

1.  `@page { size: A4; margin: 14mm }`. Letter users change it in the print dialog. `@page`
    cannot be scoped by a selector, so this applies to a print started from any tab; the
    consequence is accepted under *Other tabs* below.
2.  **Hide the shell, not a list of things inside it.** Two rules, in this order:

    ```css
    body[data-print-scope="statement"] .app-container > *:not(.main-content) { display: none }
    body[data-print-scope="statement"] .main-content > *:not([data-print="statement"]) { display: none }
    ```

    The first removes the sidebar (`<aside className="sidebar">`, `App.tsx:379`) and every other
    direct child of `.app-container` — which includes the auth-failed alert (`App.tsx`, the
    `auth_error=auth_failed` banner), a `position: fixed` full-width red banner that an
    enumerated hide list would print on every page. The second leaves only the statement in the
    content column, so the header row (`<div className="header-row">`, `App.tsx:459`) and any
    chrome added there later disappear without anyone maintaining a list. An allowlist of what to
    hide drifts every time someone adds chrome, in the same direction, silently; the inclusive
    form cannot miss a direct child.

    **Two structural preconditions, because both rules use the child combinator `>`.** The
    statement's root element must carry `data-print="statement"` **and be a direct child of
    `.main-content`** — not wrapped in a layout `<div>` or `<section>`. Wrap it and rule 2 hides
    the wrapper: a blank printed page from a tab that looks correct on screen. The Print control
    sits in the same position, as a second direct child of `.main-content` beside the statement
    root, which is what hides it — so it carries no `data-no-print` marker. The class names
    `.app-container` and `.main-content` are load-bearing for printing; renaming either in a
    later refactor silently breaks the print path. The smoke's visibility assertion is what
    catches this, and only when someone runs it.
3.  Collapse the layout: `.app-container` is a `260px 1fr` grid (`index.css:93-98`); in print it
    becomes one column, or the page prints with a 260 px gutter of nothing.
4.  Invert `--bg-dark`, `--bg-card`, `--border-color`, `--text-primary`, `--text-secondary`,
    `--text-muted` to print values; `.glass-card` loses `backdrop-filter`, `box-shadow` and its
    translucent background. **Which of these carries Goal 6:** legibility comes entirely from the
    **text** overrides, because text is always painted and does not depend on the
    background-graphics setting. The background and border overrides are redundant in the Goal 6
    sense and load-bearing in the other one: a reader who *has* enabled background graphics gets
    dark text on the dark app background without them. The two groups are not interchangeable.
    In the statement, sign is carried by the number, not by colour — `formatCurrency` renders
    negatives as `-$1,234.56`, so a greyscale printer loses nothing. (Elsewhere in the app it is
    coloured: `PortfolioDetail.tsx:286-292`.)
5.  Break rules: `tr { break-inside: avoid }`, and the headline block never splits.
6.  `[data-no-print] { display: none }` — the opt-out hatch for anything **inside** the printed
    subtree. Nothing uses it today; it is specified so a later addition can opt out without
    restructuring the tree.

#### Other tabs, and what will not invert

With the rules scoped as above, printing from the dashboard, analytics or ledger tab behaves
exactly as it does today: sidebar, navigation, charts and all. That is not designed, it is
unchanged, and it is out of scope — the statement is the tab you print from, and the only one the
smoke exercises. What changes for those tabs is `@page` alone, which no selector can scope: they
now get A4 with 14 mm margins instead of the browser default.

Two backgrounds in the app are not variables and will not invert if that scope is ever widened:
`.badge-crypto`'s background (`index.css:334`; its **color** is `var(--color-secondary)` and does
invert) and `.data-table tr:hover td` (`:252`, a hover state that cannot occur on paper and is
therefore a theoretical residual only). `.sidebar` is hardcoded `rgba(10, 15, 28, 0.95)` at
`:101`, but rule 1 hides it, so it is not a residual at all.

### Verification layers, and what each cannot cover

1.  **Vitest component test** (`frontend/src/test/components/PortfolioStatement.test.tsx`).
    jsdom does not apply print CSS, so this proves structure and formatting only.

    *Formatting* is asserted by **golden values**: a fixture carrying a value with a third decimal
    place (`1234.565`) and a second carrying more than 15 significant digits
    (`1234.56499999999999999999`), each with the expected rendered string written as a literal in
    the test. The second case is what pins the bounded residual in *Money* rather than disclaiming
    it. Anything of the form "the printed figure equals the payload value rendered through
    `formatters.ts`" cannot fail for a component that calls `formatCurrency`, and deriving the
    expected value by rounding the payload with a double re-runs the renderer's own path. No
    assertion here may compare two normalizers.

    *Structure, populated fixture*: the statement root carries `data-print="statement"` and is a
    direct child of `.main-content`; the printed subtree contains no `button`, `input`, `select`
    or `svg`; all three headline figures appear; every hook in *On-page copy* except `empty-note`
    is present with its specified text; the run-context line carries the payload's echoed
    `strategy` and `threshold_days`, with `threshold_days` absent for a non-HYBRID fixture and a
    strategy outside `FIFO`/`LIFO`/`HYBRID` not printed verbatim.

    *Structure, empty fixture* (`assets: []`): `empty-note` and `generated-at` present, and
    **absent** — `data-at`, `rounding-note`, `splits-note`, `run-context`, and all three headline
    figures.

    *Mock*: the existing `/api/portfolios/:id/tax-summary` handler (`handlers.ts:121`) gains a
    branch on portfolio id rather than a second registration. The tests genuinely at risk are the
    four that `render(<App />)` — `App.test.tsx`, `AppLedger.test.tsx`, `AppSignOut.test.tsx`,
    `AppAuthError.test.tsx` — all of which drive portfolio **id 1** and whose `Dashboard` is fed
    by this payload, so the statement fixture is pinned to an id no existing App test uses
    (**id 3**) and the default id-1 response keeps its current `assets: []`. Money fields become
    decimal strings to match the server (`db_types.rs:31`) and the handler gains `strategy`,
    `threshold_days` and `currency`. The registration-order comment at `handlers.ts:130-131`
    still applies: MSW takes the **first** match, so a specific route registered below a wildcard
    is unreachable — one handler per path, branched inside, sidesteps it.
2.  **Playwright smoke** (`frontend/e2e/statement.smoke.mjs`, following `sell-pnl.smoke.mjs`).
    Navigate to the statement tab by clicking the nav item (`.nav-item` whose accessible name is
    the statement label — Alternative G was rejected, so there is no URL to go to). Then
    `page.emulateMedia({ media: 'print' })` and assert:

    *   the statement is **visible** — non-zero box, and computed `color` on the statement root and
        on one headline figure is dark. **Text color only.** Do not assert a light background:
        Goal 6's premise is that background graphics are off, `emulateMedia` does not enable
        background painting, and `page.pdf()` defaults `printBackground: false`, so a
        background assertion either cannot be satisfied or depends on a flag this slice does not
        set. Checking the background overrides would be a second capture with
        `printBackground: true`, and is not part of Goal 6.
    *   `.sidebar` computes `display: none`. The auth alert is **not** asserted: it renders only
        from the `auth_error=auth_failed` query param, and under `AUTH_MODE=disabled` with an
        API-seeded fixture it is not in the DOM. A variant that navigates with that param may
        assert it absent-or-hidden.
    *   no horizontal overflow, and `page.pdf()` is non-empty and one page for the two-row fixture
        — **for the fixture only**, with no claim about larger portfolios (Decided item 2).
    *   **no figure is asserted.** The smoke pins prices rather than checking them: `get_price`
        returns a `historical_prices` row for today's date before it touches the network
        (`currency_service.rs:269-279`), so seeding two closes into the throwaway SQLite file
        `run.sh` already owns makes the run deterministic. Without that the figures come from a
        live provider and, when it is unreachable, the fallback returns zero and the smoke prints
        zeros and still passes.

    **Local step, not a gate.** `e2e/run.sh` hardcodes `node e2e/sell-pnl.smoke.mjs` and gains a
    target argument here; `ci.yml` has no browser job, which is issue #45. Staying local is a
    choice, not an impossibility: CI runs `ubuntu-latest`, where `npx playwright install chromium`
    is one line and `emulateMedia`/`page.pdf()` need no display server. It stays local because
    `e2e/run.sh:3-16` is not browser-only — it compiles and starts the Rust API against a
    throwaway SQLite file and starts Vite — so the job costs a full cargo build and needs seeded
    data, and because a `continue-on-error` job nobody reads would read as a pass. The real
    alternative is a blocking browser job that also runs the existing
    sell-P&L smoke, which is #45's scope; the assertions above are handed to it.

    **Auth prerequisite, fixed in this slice.** The smoke drives a real browser at a real backend,
    and every `/api` route takes `AuthedUser`, so the run needs `AUTH_MODE=disabled` — compiled
    out of release builds (`auth/mod.rs:97-101`). **`frontend/e2e/run.sh` does not set it and
    there is no `backend_rust/.env`**: with `AUTH_MODE` unset the server asks for OIDC
    (`auth/mod.rs:85`) and bails naming four missing variables (`:132-145`), so `wait_for backend`
    burns its whole 420-second budget and the smoke dies with "backend did not come up". The last
    commit to touch `run.sh` predates the auth work, so the existing harness does not currently
    start. Since this slice edits `run.sh` anyway to parameterise the target, the one-line
    `export AUTH_MODE=disabled` lands here beside the existing `DATABASE_URL` export rather than
    waiting for #45 — otherwise this slice's own acceptance step cannot run on the day it is
    written. Seeding is through the REST API as `sell-pnl.smoke.mjs:37-57` already does, which
    needs no browser login; the fixture is two holdings in one portfolio with the strategy passed
    explicitly, so the printed run context is known rather than defaulted.
3.  **A human with a print preview.** `docs/auth-testing.md` §5 is the manual browser walkthrough
    for what CI cannot check; the statement's print preview joins that list. Check: no sidebar, no
    nav, no Print control, dark text on white, one page, and the headline figures equal to the
    payload shown in devtools. Not "match the dashboard" — `Dashboard.tsx:39-41` falls back to
    `metrics.*` when a tax-summary figure is falsy, so a coincidental fallback can satisfy an
    eyeball comparison.

### Sequencing

One slice. There is no useful smaller cut: the page without the print stylesheet is a dashboard
with a different heading, and the print stylesheet without the page has nothing to style.

Two column terms. **Reaches network** = contacts a real backend or the Yahoo provider;
intercepted fetches (`test/mocks/handlers.ts`) do not count. **Moves an existing figure** = a
number already displayed on screen changes value for the same portfolio and the same data.

| Artifact | Reaches network? | Moves an existing figure? |
| :-- | :-- | :-- |
| Statement tab, print stylesheet, two payload fields surfaced | no | no |
| Component test (layer 1) | no — MSW-intercepted | no |
| Local smoke (layer 2) | **yes** — starts the Rust API, seeds through it, and reads prices from `historical_prices` rows it inserts | no |

## Target Files

### 1. `frontend/src/components/PortfolioStatement.tsx` (Create)

*   **Type / Scope:** the printable view.
*   **Proposed Changes:** renders the summary: headline block (cumulative realized since
    inception, unrealized as of today, portfolio value), the provenance block, and one row per
    holding (Decided item 1). No asset-subset filter — the only arithmetic that would make a
    filtered headline honest is the arithmetic renderers are forbidden from doing. Takes no date
    parameter. Derives no totals. No `button`, `input`, `select` or hardcoded color inside the
    printed subtree. The Print control is a **sibling** of the statement root, both direct children
    of `.main-content`, which is what hides it; it carries no `data-no-print` marker. The root
    element carries `data-print="statement"` and must not be wrapped. Sets
    `body[data-print-scope="statement"]` on mount and clears it on unmount.

### 2. `frontend/src/print.css` (Create) and `frontend/src/main.tsx` (Modify)

*   **Type / Scope:** the print stylesheet, imported after `index.css` so it wins the cascade at
    equal specificity.
*   **Proposed Changes:** the six rules in *Palette inversion*, each scoped under
    `body[data-print-scope="statement"]`, with the selectors and the two structural preconditions
    named exactly as written there.

### 3. `frontend/src/utils/decimal.ts` (Modify) and `frontend/src/types.ts` (Modify)

*   **Type / Scope:** surface the two run-context fields the payload already sends.
*   **Proposed Changes:** `normalizeTaxSummary` keeps `strategy` and `threshold_days`
    (`decimal.ts:65-67` keeps `currency` only today) and `TaxSummary` gains both as **optional**
    fields, so the existing literals in `PortfolioDetail.test.tsx` and `Dashboard.test.tsx` still
    compile under `npm run typecheck`. Additive; no existing figure moves. There is no second
    normalizer and no string-typed mirror type — Alternative C1.

### 4. `frontend/src/App.tsx` (Modify)

*   **Type / Scope:** the fifth nav item and the tab branch.
*   **Proposed Changes:** renders the statement from the `taxSummary` state `fetchPortfolioData`
    already sets, guarded the way the sibling tab guards (`activeTab === 'portfolios' && taxSummary`,
    `App.tsx:483`).
*   **Stale payload — the implementable form.** `setTaxSummary` runs once (`App.tsx:114`) and is
    never cleared, so a portfolio switch leaves the previous portfolio's summary in state until the
    new fetch resolves. "Key on the selected portfolio" is not satisfiable as written: the payload
    carries no portfolio id. The implementable rule is a `taxSummaryFor: { id: number; data:
    TaxSummary } | null` state that stores the id alongside the summary and renders only when
    `taxSummaryFor.id === selectedId`, or an equivalent clear-on-switch. A printed page with B's
    numbers under A's heading is the one failure this artifact cannot absorb quietly — it is worse
    than the zeros case because it looks right.
*   **Currency:** the header's base currency comes from the portfolio object, not the payload, and
    only because of the empty branch. *Money* states the rule once; this file list points at it
    rather than re-deriving it.
*   **Fetch timestamp:** `fetchPortfolioData` records the time beside `setTaxSummary` so the
    `data-at` line has a source.

### 5. `frontend/src/test/components/PortfolioStatement.test.tsx` (Create) and `frontend/src/test/mocks/handlers.ts` (Modify)

*   **Type / Scope:** structural tests and the mock branch.
*   **Proposed Changes:** the assertions listed under *Verification layers*. The mock at
    `handlers.ts:121` returns `total_realized_pnl: 0` as a JSON **number** while the backend always
    sends strings (`analytics.rs:95-97`, `db_types.rs:31`), so the fixture carries decimal strings
    plus the two echoed context fields, is pinned to portfolio **id 3** to leave the four
    `render(<App />)` tests on id 1 untouched, and the default id-1 response keeps its current
    `assets: []`.

### 6. `frontend/e2e/statement.smoke.mjs` (Create), `frontend/e2e/run.sh` (Modify), `frontend/package.json` (Modify)

*   **Type / Scope:** the browser smoke, and the parameterisation it needs.
*   **Proposed Changes:** as in *Verification layers* layer 2. `run.sh` hardcodes
    `node e2e/sell-pnl.smoke.mjs`; it gains a target argument defaulting to the existing smoke, and
    gains `export AUTH_MODE=disabled` beside its existing `DATABASE_URL` export. `package.json`
    gains `e2e:statement`. **Not wired into CI** — issue #45.

### 7. Deleted — no print-CSS grep gate

Carried from the parent design. A grep asserting a stylesheet mentions some selectors goes green
while the page still prints badly. The Playwright smoke is the check; see Alternative B for why no
grep ships.

## Risks

| Risk | Evidence | Mitigation |
| :-- | :-- | :-- |
| **The print path ships unverified.** No CI browser job; the smoke is a local step. | `ci.yml` has no browser job; issue #45 open | Selectors and smoke assertions are named here and handed to #45. The manual print preview is listed in `docs/auth-testing.md` §5. |
| **The e2e harness does not currently start**, so the smoke's acceptance step cannot run. | `run.sh` sets no `AUTH_MODE`; the server bails naming four OIDC variables (`auth/mod.rs:132-145`) | This slice adds `export AUTH_MODE=disabled` to `run.sh` (Target Files 6), because it edits that file anyway. |
| **Palette override misses a hardcoded color inside the statement.** | `SellPnlModal.tsx:132` uses inline `background: '#121929'`; `PortfolioDetail.tsx:286-292` uses `var(--color-success)`, which inverts but is not one of the six variables step 4 lists | The statement uses no hardcoded colors and no success/danger coloring — sign is in the number. The smoke's text-color check catches a hardcoded **text** color and **cannot** see a hardcoded background; that residual is accepted rather than gated, for the reason in Alternative B's scope note. |
| **A printed figure differs from the CSV's by one unit of the last displayed digit.** | The CSV is full-precision (`docs/superpowers/reviews/2026-11-05-sell-pnl-breakdown.md:18`); `roi` is measured at 28 fractional digits (`:22`); `normalizeTaxSummary` converts money to `number` before any statement code runs (`decimal.ts:65-67`) | Goal 3 declares the rounding on the page; *Money* bounds the residual to one unit of the last displayed digit and Verification layer 1 pins it with a >15-significant-digit golden. |
| **A third renderer of one payload, with nothing comparing it to the second.** | `PortfolioDetail.tsx:238-292` renders the same `taxSummary.assets`; `Dashboard.tsx:40-45` renders the same headline figures | Accepted knowingly (Alternative D). The statement is a leaf with no interactivity. If the two tables ever need to agree, the check is a comparison test between them, and it is not written here. |
| **A4 on Letter hardware.** | `@page { size: A4 }`, and `@page` cannot be scoped, so other tabs' paper size changes too | Stated in the dialog, not hidden. The owner's portfolios are EUR-denominated, which is a hint about likely paper preference and nothing more. |
| **Statement read as a quarterly figure.** | The undated endpoint has no window | Goal 1's label requirement: "cumulative since inception" is on the page, not implied. |
| **Empty portfolio prints zeros.** | `analytics.rs:45-51` returns zeros with no `currency` | Goal 7 and *Empty portfolio*: heading, printed date and the empty note only. The negative list is asserted. |
| **A stale payload prints under the wrong portfolio name.** | `setTaxSummary` is called once (`App.tsx:114`) and never cleared | Target Files 4 gives the implementable form (store the id with the summary; render only on match). |
| **A print started from another tab is not designed.** | The hide rules are scoped to the statement tab, so other tabs print as today | Out of scope and stated under *Other tabs*. The one change for those tabs is `@page`, which no selector can scope. |

## Decided during drafting

Three items were open at first draft. Each is settled here, with what it trades away.

1.  **The statement lists holdings.** The headline figures are the point, and a 40-row
    table is a second page and a second set of numbers to reconcile against the CSV. It
    lists them anyway: a statement without holdings is a receipt, and the owner's use case
    is handing something to a person who will ask "which positions?". The reconciliation
    cost is real and is bounded by Goal 3 — every printed row is a payload value rounded
    once at display, so a row cannot disagree with the CSV except by the rounding the page
    declares. What the rows can do is run to a second page, which is a layout fact, not a
    correctness one.
2.  **The smoke asserts one page for its own fixture and says nothing about larger
    portfolios.** One page for a two-row fixture is checkable. A bound that holds for 50
    holdings is a layout claim nobody has measured, and an assertion that fails on the
    owner's real portfolio but passes in CI is an assertion that teaches people to ignore
    the smoke. The page-break rules still ship (`tr { break-inside: avoid }`); they are
    simply not claimed to produce a page count.
3.  **The strategy line prints what the server echoed.** The payload carries `strategy`,
    `threshold_days` and `currency` (`analytics.rs`, the tail of the tax-summary response), and
    the route passes the query values straight to `calculate_lots`, so the echoed value is the
    value used. The only gap was that `normalizeTaxSummary` dropped two of the three fields, which
    Target Files 3 fixes. The wording precedent is already on screen at `SellPnlModal.tsx:210`
    (`realized · FIFO (threshold 30d)`). The narrower claim this slice can make, and the reason the
    run-context line is described as "as the app sent it" rather than "as the engine used it", is
    in *What "splits" and the strategy line mean*.

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
*   **Reason for rejection**: a gate that can pass while the feature is broken converts
    "unverified" into "verified and therefore fine". The Playwright smoke is the gate; it is
    waiting on issue #45, the same blocker as the existing sell-P&L smoke.
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
    payload.
*   **Cons**: the existing table is exactly what Goal 2 forbids. It has buttons, expanders, an
    Actions column, and inline `var(--color-success)` colors on the P&L cells
    (`PortfolioDetail.tsx:286-292`); it takes its currency from `portfolio?.currency || 'USD'`
    (`:261`) rather than from the payload. Making it print well means suppressing most of it
    and re-deriving what the leaf component gets for free.
*   **Reason for rejection**: a dedicated leaf is the cheaper artifact. The cost is recorded
    plainly: **this slice adds a third renderer of one payload.** The two tables will disagree
    about columns, currency source, and sign and rounding conventions unless something compares
    them, and neither the smoke nor the golden does.

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

## Verification commands

*   `npm test`, `npm run lint`, `npm run typecheck` from `frontend/`.
*   `npm run e2e:statement` from `frontend/` — **local step, not a gate**. Needs a chromium
    binary and fonts; on NixOS,
    `nix-shell -p chromium -p liberation_ttf --run './e2e/run.sh statement'`. It also needs the
    `AUTH_MODE=disabled` export described in *Verification layers* layer 2, which lands in
    `run.sh` with this slice.
*   Manual: open the statement, print preview, confirm no sidebar, no nav, no Print control, dark
    text on white, one page, and that the headline figures equal the payload shown in devtools —
    not "match the dashboard", for the fallback reason given in *Verification layers* layer 3.
    Added to the manual browser walkthrough in `docs/auth-testing.md` §5.
*   Cross-check **once the tax report ships** — it does not exist yet, and its date semantics are
    still open in the parent design: same portfolio, same moment, the statement's cumulative
    realized equals the CSV's cumulative realized column rounded the way `formatters.ts` rounds
    it, and the page carries the line saying it was rounded. Until then the shipped equality is the
    golden-value test in layer 1, evaluated against the payload.
