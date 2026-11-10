# Portfolio Statement (Printable) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a fifth tab that renders the portfolio's cumulative realized and current unrealized P&L as a printable A4 document, inverted to a light palette for paper.

**Architecture:** Pure frontend over the existing `GET /api/portfolios/<id>/tax-summary` payload. A new leaf component renders cached state — it issues no request of its own — and a print stylesheet inverts the app's CSS variables inside `@media print`, scoped to the statement tab with `:has()` so the other four tabs print exactly as they do today. All totals come from the backend; the renderer is forbidden from summing anything.

**Tech Stack:** React 18 + Vite, TypeScript, plain CSS (no framework), Vitest + Testing Library, Playwright (local smoke only), Rust/Rocket backend untouched by this plan.

**Spec:** `docs/superpowers/specs/2026-11-09-portfolio-statement-print.md` — this plan implements it. Where this plan and the spec disagree, the spec wins; report the disagreement rather than silently resolving it.

## Global Constraints

- **No new dependency.** Nothing is added to `frontend/package.json` except one npm script. No PDF library, no print library, no date library.
- **Money is formatted, never recomputed.** Every figure goes through `formatCurrency` / `formatPercent` from `frontend/src/utils/formatters.ts`. No renderer may add, average, or otherwise combine payload figures.
- **The backend owns all totals.** If a number looks like it needs summing, that is a spec error — stop and report it.
- **Copy is literal and English.** The strings in *On-page copy* of the spec are requirements. Assertions match `textContent`. Write `P&L`, not `P&amp;L`.
- **Hooks follow the repo's `data-testid` convention.** The single exception is `data-print="statement"`, which is load-bearing in CSS.
- **Tests reach no network.** Component tests use a prop fixture, never the shared MSW handler at `frontend/src/test/mocks/handlers.ts`.
- **`frontend/test/setup.ts:3` pins `process.env.TZ = 'UTC'`.** Any test that reads a date must also freeze the clock; a frozen clock without a pinned zone is reproducible but not deterministic.
- **All frontend commands run from `frontend/`.** Gate set: `npm run test`, `npm run lint`, `npm run typecheck`.
- **Branch:** `feat/report-export`. Commit after every task.

## Review Focus

The five things no task's own tests fully cover, most likely to bite first. Each gets a pinning test in the task that owns the code.

1. **A wrapper element around the statement root prints a blank page.** Both hide rules use the child combinator `>`. Wrap the statement in a layout `<div>` and rule 2 hides the wrapper — the tab looks perfect on screen and prints nothing. Pinned by Task 4's structural assertion that the statement root's parent is `.main-content`.
2. **A portfolio switch leaves the previous portfolio's summary on screen.** `setTaxSummary` runs once and is never cleared, so B's numbers can render under A's heading. Unlike a zeros case this looks correct. Pinned by Task 4's stale-payload test.
3. **The print stylesheet is inert when the statement does not render.** `:has()` matches only when `[data-print="statement"]` exists in the tree, so on any other tab nothing is hidden — which is the design, but it also means a broken statement render silently reverts to printing the whole dashboard. Pinned by Task 3's negative assertion and Task 5's visibility assertion.
4. **A hardcoded colour inside the statement does not invert.** `.data-table td`'s border is a literal `rgba(255,255,255,0.03)` (`index.css:247`), not a variable, so the `--border-color` override does not reach it and the table half-inverts. Pinned by Task 3's explicit `.statement-table td` override and Task 5's text-colour assertion.
5. **The empty portfolio prints figures it does not have.** The empty branch must print exactly two lines. Pinned by Task 2's empty-fixture test, which asserts absence, not just presence.

---

## Task 1: Surface the run-context fields the payload already sends

The backend already echoes `strategy` and `threshold_days` (`backend_rust/src/api_routes/analytics.rs:98-100`). `normalizeTaxSummary` drops both — it keeps `currency` and nothing else at the top level (`frontend/src/utils/decimal.ts:63`). This task recovers them. Additive: no existing figure moves, and both fields are optional so every existing test literal still compiles.

**Files:**
- Modify: `frontend/src/types.ts` (the `TaxSummary` interface)
- Modify: `frontend/src/utils/decimal.ts:59-67` (`normalizeTaxSummary`)
- Test: `frontend/src/utils/decimal.test.ts` (extend; create the `describe` block if absent)

**Interfaces:**
- Consumes: nothing.
- Produces: `TaxSummary.strategy?: string` and `TaxSummary.threshold_days?: number`, populated by `normalizeTaxSummary`. Task 2 reads both.

- [ ] **Step 1: Write the failing test**

Add to `frontend/src/utils/decimal.test.ts`:

```ts
import { normalizeTaxSummary } from './decimal';

describe('normalizeTaxSummary run-context fields', () => {
  it('keeps strategy and threshold_days when the payload echoes them', () => {
    const s = normalizeTaxSummary({
      currency: 'EUR', strategy: 'HYBRID', threshold_days: 45,
      total_portfolio_value: '10.00', total_realized_pnl: '1234.565',
      total_unrealized_pnl: '0.00', assets: [],
    });
    expect(s.strategy).toBe('HYBRID');
    expect(s.threshold_days).toBe(45);
  });

  it('leaves both undefined when the payload omits them (the empty-portfolio branch)', () => {
    const s = normalizeTaxSummary({
      total_portfolio_value: 0, total_realized_pnl: 0, total_unrealized_pnl: 0, assets: [],
    });
    expect(s.strategy).toBeUndefined();
    expect(s.threshold_days).toBeUndefined();
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd frontend && npx vitest run src/utils/decimal.test.ts`
Expected: FAIL — `s.strategy` is `undefined` in the first case.

- [ ] **Step 3: Add the optional fields to the type**

In `frontend/src/types.ts`, inside `export interface TaxSummary { ... }`, after the `currency` line:

```ts
  strategy?: string;
  threshold_days?: number;
```

They are optional on purpose. `PortfolioDetail.test.tsx:25-63` and `Dashboard.test.tsx:38-42` build `TaxSummary` literals without them; making them required would break `npm run typecheck` for no benefit, and the empty-portfolio response genuinely carries neither.

- [ ] **Step 4: Keep them in the normalizer**

In `frontend/src/utils/decimal.ts`, inside the object returned by `normalizeTaxSummary`, immediately after the `currency:` line:

```ts
    strategy: typeof r.strategy === 'string' ? r.strategy : undefined,
    threshold_days: typeof r.threshold_days === 'number' ? r.threshold_days : undefined,
```

Note the shape difference from the other fields: `str()` and `num()` default to `''` and `0`, and a defaulting `strategy` would render as `Portfolio · USD · ` with a dangling separator. `undefined` is the correct absence signal here, so these two do not use the helpers.

- [ ] **Step 5: Run the test to verify it passes**

Run: `cd frontend && npx vitest run src/utils/decimal.test.ts`
Expected: PASS.

- [ ] **Step 6: Verify nothing existing moved**

Run: `cd frontend && npm run test && npm run typecheck`
Expected: all suites pass; `tsc --noEmit` clean. If a `TaxSummary` literal now fails, the fields were added as required rather than optional.

- [ ] **Step 7: Commit**

```bash
git add frontend/src/types.ts frontend/src/utils/decimal.ts frontend/src/utils/decimal.test.ts
git commit -m "feat(frontend): surface strategy and threshold_days from the tax-summary payload"
```

---

## Task 2: The statement component

The printable view, and the component test that pins its structure and formatting. This task does not wire the component into the app — Task 4 does that. Rendering it in isolation is what makes the empty branch and the golden assertable without a router or a mock server.

**Files:**
- Create: `frontend/src/components/PortfolioStatement.tsx`
- Create: `frontend/src/test/components/PortfolioStatement.test.tsx`

**Interfaces:**
- Consumes: `TaxSummary` and `Portfolio` from `frontend/src/types.ts` (including Task 1's two optional fields); `formatCurrency` and `formatPercent` from `frontend/src/utils/formatters.ts`.
- Produces: `PortfolioStatement` default-exported, props `{ taxSummary: TaxSummary; portfolio: Portfolio; fetchedAt: Date }`. Task 4 renders it. Hooks produced: `data-print="statement"`, `.statement-headline`, `.statement-table`, `figure-value`, `figure-realized`, `figure-unrealized`, `holdings`, plus the seven `data-testid` copy hooks in the spec's copy table.

- [ ] **Step 1: Write the failing component test**

Create `frontend/src/test/components/PortfolioStatement.test.tsx`:

```tsx
import { render, screen } from '@testing-library/react';
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import PortfolioStatement from '../../components/PortfolioStatement';
import type { Portfolio, TaxSummary } from '../../types';

const portfolio: Portfolio = {
  id: 1, name: 'Long-Term', currency: 'EUR', assets: [],
};

const summary = (over: Partial<TaxSummary> = {}): TaxSummary => ({
  currency: 'EUR',
  strategy: 'HYBRID',
  threshold_days: 45,
  total_portfolio_value: 10400,
  total_realized_pnl: 1234.565,
  total_unrealized_pnl: -800,
  assets: [{
    symbol: 'AAPL', asset_type: 'stock', current_shares: 10, average_cost: 100,
    current_price: 120, total_cost: 1000, market_value: 1200,
    unrealized_pnl: 200, unrealized_roi: 0.2, realized_pnl: 1234.565,
    tax_lots: [],
  }],
  ...over,
});

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date('2026-11-09T10:30:00Z'));
});
afterEach(() => { vi.useRealTimers(); });

describe('PortfolioStatement, populated', () => {
  const fetchedAt = new Date('2026-11-09T10:30:00Z');
  const renderIt = () => render(
    <PortfolioStatement taxSummary={summary()} portfolio={portfolio} fetchedAt={fetchedAt} />
  );

  it('renders the statement root the print CSS keys on', () => {
    renderIt();
    expect(document.querySelector('[data-print="statement"]')).not.toBeNull();
  });

  it('formats money through Intl at the payload currency — the golden', () => {
    renderIt();
    // 1234.565 is quantised at display; the nearest double sits above the midpoint.
    expect(screen.getByTestId('figure-realized').textContent).toBe('€1,234.57');
  });

  it('carries the three figures, each non-empty', () => {
    renderIt();
    for (const id of ['figure-value', 'figure-realized', 'figure-unrealized']) {
      expect(screen.getByTestId(id).textContent).not.toBe('');
    }
  });

  it('prints the literal copy with the two clocks distinguished', () => {
    renderIt();
    expect(screen.getByTestId('label-realized').textContent)
      .toBe('Realized P&L — cumulative since inception');
    expect(screen.getByTestId('generated-at').textContent).toBe('Printed Nov 9, 2026');
    expect(screen.getByTestId('data-at').textContent).toBe('Data fetched Nov 9, 2026, 10:30 AM');
    expect(screen.getByTestId('rounding-note').textContent)
      .toBe('Figures are rounded for display from full-precision calculations.');
    expect(screen.getByTestId('splits-note').textContent)
      .toBe('Split-adjusted history is as recorded when this page was loaded; '
        + 'late-recorded splits can change past figures.');
  });

  it('names the strategy and threshold in the run context', () => {
    renderIt();
    expect(screen.getByTestId('run-context').textContent)
      .toBe('Long-Term · EUR · HYBRID (threshold 45d)');
  });

  it('omits the threshold for a non-HYBRID strategy', () => {
    render(
      <PortfolioStatement
        taxSummary={summary({ strategy: 'FIFO', threshold_days: undefined })}
        portfolio={portfolio} fetchedAt={fetchedAt}
      />
    );
    expect(screen.getByTestId('run-context').textContent).toBe('Long-Term · EUR · FIFO');
  });

  it('renders nine columns and one row per holding', () => {
    renderIt();
    const headers = Array.from(
      document.querySelectorAll('[data-testid="holdings"] thead th')
    ).map((th) => th.textContent);
    expect(headers).toEqual([
      'Symbol', 'Type', 'Quantity', 'Avg cost', 'Price',
      'Market value', 'Realized', 'Unrealized', 'Unrealized ROI',
    ]);
    expect(document.querySelectorAll('[data-testid="holdings"] tbody tr')).toHaveLength(1);
  });

  it('keeps interactive and painted elements out of the printed subtree', () => {
    renderIt();
    expect(
      document.querySelectorAll(
        '[data-print="statement"] button, [data-print="statement"] input, '
        + '[data-print="statement"] select, [data-print="statement"] svg'
      )
    ).toHaveLength(0);
  });
});

describe('PortfolioStatement, empty portfolio', () => {
  it('prints two lines and nothing that looks like a figure', () => {
    render(
      <PortfolioStatement
        taxSummary={summary({ assets: [], strategy: undefined, threshold_days: undefined })}
        portfolio={portfolio}
        fetchedAt={new Date('2026-11-09T10:30:00Z')}
      />
    );
    expect(screen.getByTestId('empty-note').textContent)
      .toBe('No holdings in this portfolio on Nov 9, 2026.');
    expect(screen.getByTestId('generated-at')).toBeDefined();
    for (const id of ['data-at', 'rounding-note', 'splits-note', 'run-context',
                      'figure-value', 'figure-realized', 'figure-unrealized']) {
      expect(screen.queryByTestId(id)).toBeNull();
    }
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd frontend && npx vitest run src/test/components/PortfolioStatement.test.tsx`
Expected: FAIL — cannot resolve `../../components/PortfolioStatement`.

- [ ] **Step 3: Write the component**

Create `frontend/src/components/PortfolioStatement.tsx`:

```tsx
import { formatCurrency, formatPercent } from '../utils/formatters';
import type { Portfolio, TaxSummary } from '../types';

interface PortfolioStatementProps {
  taxSummary: TaxSummary;
  portfolio: Portfolio;
  fetchedAt: Date;
}

const DATE_OPTS: Intl.DateTimeFormatOptions = { month: 'short', day: 'numeric', year: 'numeric' };
const DATETIME_OPTS: Intl.DateTimeFormatOptions = { ...DATE_OPTS, hour: '2-digit', minute: '2-digit' };
const QTY_OPTS: Intl.NumberFormatOptions = { maximumFractionDigits: 6 };

export default function PortfolioStatement(
  { taxSummary, portfolio, fetchedAt }: PortfolioStatementProps,
) {
  const currency = taxSummary.currency;
  const printDate = new Date().toLocaleDateString('en-US', DATE_OPTS);
  const empty = taxSummary.assets.length === 0;

  // The empty-portfolio response carries no strategy, so run-context is absent rather
  // than rendered with a dangling separator.
  let runContext: string | null = null;
  if (!empty && taxSummary.strategy) {
    runContext = `${portfolio.name} · ${portfolio.currency} · ${taxSummary.strategy}`;
    if (taxSummary.strategy === 'HYBRID' && taxSummary.threshold_days !== undefined) {
      runContext += ` (threshold ${taxSummary.threshold_days}d)`;
    }
  }

  return (
    <section data-print="statement" className="statement">
      <header className="statement-header">
        <h1>Portfolio Statement</h1>
        {runContext && <p data-testid="run-context">{runContext}</p>}
        <p data-testid="generated-at">Printed {printDate}</p>
        {!empty && (
          <p data-testid="data-at">Data fetched {fetchedAt.toLocaleString('en-US', DATETIME_OPTS)}</p>
        )}
      </header>

      {empty ? (
        <p data-testid="empty-note">No holdings in this portfolio on {printDate}.</p>
      ) : (
        <>
          <div className="statement-headline">
            <div className="statement-figure">
              <span>Portfolio value</span>
              <strong data-testid="figure-value">
                {formatCurrency(taxSummary.total_portfolio_value, currency)}
              </strong>
            </div>
            <div className="statement-figure">
              <span data-testid="label-realized">Realized P&L — cumulative since inception</span>
              <strong data-testid="figure-realized">
                {formatCurrency(taxSummary.total_realized_pnl, currency)}
              </strong>
            </div>
            <div className="statement-figure">
              <span>Unrealized P&L — as of today</span>
              <strong data-testid="figure-unrealized">
                {formatCurrency(taxSummary.total_unrealized_pnl, currency)}
              </strong>
            </div>
          </div>

          <table className="data-table statement-table" data-testid="holdings">
            <thead>
              <tr>
                <th>Symbol</th><th>Type</th><th>Quantity</th><th>Avg cost</th><th>Price</th>
                <th>Market value</th><th>Realized</th><th>Unrealized</th><th>Unrealized ROI</th>
              </tr>
            </thead>
            <tbody>
              {taxSummary.assets.map((a) => (
                <tr key={`${a.symbol}-${a.asset_type}`}>
                  <td>{a.symbol}</td>
                  <td>{a.asset_type}</td>
                  <td>{a.current_shares.toLocaleString(undefined, QTY_OPTS)}</td>
                  <td>{formatCurrency(a.average_cost, currency)}</td>
                  <td>{formatCurrency(a.current_price, currency)}</td>
                  <td>{formatCurrency(a.market_value, currency)}</td>
                  <td>{formatCurrency(a.realized_pnl, currency)}</td>
                  <td>{formatCurrency(a.unrealized_pnl, currency)}</td>
                  <td>{formatPercent(a.unrealized_roi)}</td>
                </tr>
              ))}
            </tbody>
          </table>

          <footer className="statement-notes">
            <p data-testid="rounding-note">
              Figures are rounded for display from full-precision calculations.
            </p>
            <p data-testid="splits-note">
              Split-adjusted history is as recorded when this page was loaded; late-recorded
              splits can change past figures.
            </p>
          </footer>
        </>
      )}
    </section>
  );
}
```

Three things this deliberately does not do, because the spec forbids them: it derives no total (every figure is read straight off the payload), it applies no asset-subset filter, and it colours nothing — sign is in the number, which is what makes a greyscale printer lose nothing.

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd frontend && npx vitest run src/test/components/PortfolioStatement.test.tsx`
Expected: PASS, 9 tests.

If the golden fails, do not "fix" it by computing the expected value in the test. The whole point is a literal that a future change to `formatters.ts` breaks loudly.

- [ ] **Step 5: Run the full frontend gate**

Run: `cd frontend && npm run test && npm run lint && npm run typecheck`
Expected: all green.

- [ ] **Step 6: Commit**

```bash
git add frontend/src/components/PortfolioStatement.tsx frontend/src/test/components/PortfolioStatement.test.tsx
git commit -m "feat(frontend): add the printable portfolio statement component"
```

---

## Task 3: The print stylesheet

Five jobs, in the spec's order: `@page`, the two inclusive hide rules, the layout collapse, the palette inversion, and the break rules. The hide rules are scoped with `:has()` so the other four tabs print unchanged.

**Files:**
- Create: `frontend/src/print.css`
- Modify: `frontend/src/main.tsx:3` (add the import after `index.css`)
- Test: `frontend/src/test/components/PrintCss.test.ts`

**Interfaces:**
- Consumes: the class names and hooks Task 2 emits (`.statement-headline`, `.statement-table`, `[data-print="statement"]`), and the existing `.app-container` / `.main-content` / `.glass-card` from `index.css`.
- Produces: nothing consumed by later tasks. The smoke (Task 5) asserts against these rules.

- [ ] **Step 1: Write the failing stylesheet test**

jsdom does not apply `@media print`, so this test reads the file as text and asserts the rules exist with the values the smoke will compare against. It is a contract test between the stylesheet and the smoke's colour assertion — not a substitute for either.

Create `frontend/src/test/components/PrintCss.test.ts`:

```ts
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, it, expect } from 'vitest';

const css = readFileSync(resolve(__dirname, '../../print.css'), 'utf8');

describe('print.css', () => {
  it('scopes both hide rules with :has() so the other tabs print unchanged', () => {
    expect(css).toMatch(/\.app-container:has\(\[data-print="statement"\]\)\s*>\s*\*:not\(\.main-content\)/);
    expect(css).toMatch(/\.main-content:has\(\[data-print="statement"\]\)\s*>\s*\*:not\(\[data-print="statement"\]\)/);
  });

  it('sets the palette the smoke compares against, literally', () => {
    for (const decl of [
      '--text-primary: #111111', '--text-secondary: #333333', '--text-muted: #555555',
      '--bg-dark: #ffffff', '--bg-card: #ffffff', '--border-color: #333333',
    ]) {
      expect(css).toContain(decl);
    }
  });

  it('overrides the table body border, which is a literal in index.css and not a variable', () => {
    expect(css).toMatch(/\.statement-table\s+td\s*\{[^}]*border-bottom:\s*1px solid #333333/);
  });

  it('collapses the two-column grid and keeps the headline off a page break', () => {
    expect(css).toMatch(/grid-template-columns:\s*1fr/);
    expect(css).toMatch(/\.statement-headline\s*\{[^}]*break-inside:\s*avoid/);
    expect(css).toMatch(/tr\s*\{[^}]*break-inside:\s*avoid/);
  });

  it('declares A4 once, unscoped, because @page takes no selector', () => {
    expect(css).toMatch(/@page\s*\{\s*size:\s*A4;\s*margin:\s*14mm/);
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd frontend && npx vitest run src/test/components/PrintCss.test.ts`
Expected: FAIL — `print.css` does not exist.

- [ ] **Step 3: Write the stylesheet**

Create `frontend/src/print.css`:

```css
/*
 * Print palette and hide rules for the printable statement. Imported after index.css
 * in main.tsx so it wins the cascade at equal specificity.
 *
 * The hide rules are scoped with :has() on purpose. Unscoped, `.main-content > *:not(...)`
 * hides every direct child of the content column on the dashboard, analytics and ledger
 * tabs, because nothing there carries the attribute — three of four tabs would print blank.
 * :has() is baseline (Chrome 105 / Safari 15.4 / Firefox 121); the repo sets no
 * browserslist override.
 */

/* @page cannot take a selector, so this applies to a print started from any tab. */
@page { size: A4; margin: 14mm; }

@media print {
  /* 1. Hide the shell, not a list of things inside it. An enumerated hide list drifts
        every time someone adds chrome; the inclusive form cannot miss a direct child.
        Rule one removes the sidebar and the auth-failed banner (a position:fixed
        full-width banner an enumerated list would print on every page). Rule two leaves
        only the statement in the content column, which is also what hides the Print
        control — it is a sibling of the statement root, not inside it.
        Both use `>`, so the statement root must be a DIRECT child of .main-content.
        Wrap it in a layout div and this prints a blank page from a tab that looks
        correct on screen. */
  .app-container:has([data-print="statement"]) > *:not(.main-content) { display: none; }
  .main-content:has([data-print="statement"]) > *:not([data-print="statement"]) { display: none; }

  /* 2. Collapse the 260px 1fr grid, or the page prints with a 260px gutter of nothing. */
  .app-container:has([data-print="statement"]) { grid-template-columns: 1fr; }

  /* 3. Invert the palette. Legibility comes entirely from the TEXT overrides: text is
        always painted and does not depend on the "background graphics" setting. The
        background and border overrides are redundant in that sense and load-bearing in
        the other one — a reader who HAS enabled background graphics otherwise gets dark
        text on the dark app background. The two groups are not interchangeable. */
  :root {
    --bg-dark: #ffffff;
    --bg-card: #ffffff;
    --border-color: #333333;
    --text-primary: #111111;
    --text-secondary: #333333;
    --text-muted: #555555;
  }

  .glass-card {
    backdrop-filter: none;
    box-shadow: none;
    background: #ffffff;
  }

  /* .data-table td's own border is a literal rgba(255,255,255,0.03) (index.css:247),
        not a variable, so --border-color never reaches it. Without this the header rule
        inverts and every body rule stays invisible: a half-inverted table. */
  .statement-table td { border-bottom: 1px solid #333333; }

  /* 4. Break rules. */
  tr { break-inside: avoid; }
  .statement-headline { break-inside: avoid; }
}
```

- [ ] **Step 4: Import it after `index.css`**

In `frontend/src/main.tsx`, immediately after the existing `import './index.css';` at `:3`:

```tsx
import './print.css';
```

Source order is doing the work here, not the file boundary — a `@media print` block appended to `index.css` would win just as reliably. The separate file is for organisation, and its cost is that the print palette is not sitting next to the palette it inverts.

- [ ] **Step 5: Run the test to verify it passes**

Run: `cd frontend && npx vitest run src/test/components/PrintCss.test.ts`
Expected: PASS, 5 tests.

- [ ] **Step 6: Run the full frontend gate**

Run: `cd frontend && npm run test && npm run lint && npm run typecheck`
Expected: all green.

- [ ] **Step 7: Commit**

```bash
git add frontend/src/print.css frontend/src/main.tsx frontend/src/test/components/PrintCss.test.ts
git commit -m "feat(frontend): print stylesheet inverts the palette for the statement tab"
```

---

## Task 4: Wire the fifth tab

The nav item, the tab branch, and the `taxMeta` record that stops a portfolio switch leaving stale numbers on the page. This is the task where the structural preconditions from Task 3 either hold or silently produce a blank print.

**Files:**
- Modify: `frontend/src/App.tsx` (imports `:2-13`; state `:35-45`; `fetchPortfolioData` `:111-114`; nav list `:386-398`; tab branch region `:472-500`)
- Test: `frontend/src/test/components/AppStatement.test.tsx` (beside `AppLedger.test.tsx`, which is the pattern to copy)

**Interfaces:**
- Consumes: `PortfolioStatement` from Task 2; the hooks Task 3's CSS keys on.
- Produces: tab id `'statement'`, nav label `Printable Statement`, `data-testid="print-button"`, and App state `taxMeta: { portfolioId: number; fetchedAt: Date } | null`.

- [ ] **Step 1: Write the failing test**

Create `frontend/src/test/components/AppStatement.test.tsx`. This is an App-level test, so it renders the whole `App` against the shared MSW handlers — copy the setup shape from `frontend/src/test/components/AppLedger.test.tsx:1-15`. The mock list at `test/mocks/handlers.ts:6-36` holds two portfolios: `1 / Test Portfolio` and `2 / Retirement Fund`, and `App.tsx:89` auto-selects the first.

```tsx
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { http, HttpResponse } from 'msw'
import App from '../../App'
import { server } from '../mocks/server'

describe('statement tab', () => {
  beforeEach(() => { vi.stubGlobal('alert', vi.fn()); vi.stubGlobal('confirm', vi.fn(() => true)) })
  afterEach(() => vi.unstubAllGlobals())

  it('navigates by the nav item a person can actually see', async () => {
    const user = userEvent.setup()
    const { container } = render(<App />)
    // These are <li onClick> with no role, so there is no accessible name to query.
    // getByText is the idiom the smoke uses too (sell-pnl.smoke.mjs:107).
    await user.click(await screen.findByText('Printable Statement'))
    await waitFor(() => expect(screen.getByTestId('print-button')).toBeInTheDocument())
    expect(container.querySelector('[data-print="statement"]')).not.toBeNull()
  })

  it('renders the statement root as a DIRECT child of .main-content', async () => {
    const user = userEvent.setup()
    const { container } = render(<App />)
    await user.click(await screen.findByText('Printable Statement'))
    await waitFor(() => {
      const root = container.querySelector('[data-print="statement"]')
      expect(root).not.toBeNull()
      // Both print hide rules use the child combinator. A wrapper here prints a blank
      // page from a tab that looks correct on screen.
      expect(root!.parentElement!.className).toContain('main-content')
    })
  })

  it('renders nothing while the loaded summary belongs to another portfolio', async () => {
    const user = userEvent.setup()
    // Portfolio 2's tax-summary never resolves, so after the switch taxSummary still
    // holds portfolio 1's figures while selectedId has moved on. This is the exact
    // state the guard exists for: setTaxSummary runs once and is never cleared, and a
    // page with B's numbers under A's heading looks correct.
    server.use(http.get('/api/portfolios/2/tax-summary', () => new Promise(() => {})))
    const { container } = render(<App />)
    await user.click(await screen.findByText('Printable Statement'))
    await waitFor(() =>
      expect(container.querySelector('[data-print="statement"]')).not.toBeNull())

    await user.selectOptions(screen.getByLabelText('ACTIVE PORTFOLIO'), 'Retirement Fund')

    await waitFor(() =>
      expect(container.querySelector('[data-print="statement"]')).toBeNull())
    expect(screen.queryByTestId('print-button')).toBeNull()
  })
})
```

The third test is the one that matters. If it passes for the wrong reason — because the statement never rendered in the first place — the first test would also fail, so the pair pins each other.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd frontend && npx vitest run src/test/components/AppStatement.test.tsx`
Expected: FAIL — `Unable to find an element with the text: Printable Statement`.

- [ ] **Step 3: Add the import and the state**

In `frontend/src/App.tsx`, add `Printer` to the `lucide-react` import block (`:2-13`) and import the component:

```tsx
  Printer,
} from 'lucide-react';
import PortfolioStatement from './components/PortfolioStatement';
```

Add beside the existing tax state (after `const [taxSummary, setTaxSummary] = ...`):

```tsx
  // setTaxSummary runs once and is never cleared, so a portfolio switch leaves the
  // previous portfolio's summary in state until the new fetch resolves. A page with B's
  // numbers under A's heading is the one failure this artifact cannot absorb quietly --
  // unlike the zeros case, it looks right. The payload carries no portfolio id, so
  // record the id beside it and guard the statement branch on the match. Same shape as
  // sellPnl at :68, and for the reason that comment gives: five places change
  // selectedId, and a reset that has to be remembered in each one is a reset that gets
  // forgotten. Derived, not cleared from an effect.
  const [taxMeta, setTaxMeta] = useState<{ portfolioId: number; fetchedAt: Date } | null>(null);
```

- [ ] **Step 4: Record the id and the timestamp where the summary is set**

In `fetchPortfolioData`, replace the single `setTaxSummary(normalizeTaxSummary(taxData));` line (`:114`) with:

```tsx
      const normalizedTax = normalizeTaxSummary(taxData);
      setTaxSummary(normalizedTax);
      // The data-at line needs a source, and the statement renders cached state, so the
      // fetch time is captured here rather than at render time.
      setTaxMeta({ portfolioId: id, fetchedAt: new Date() });
```

- [ ] **Step 5: Add the nav item**

In the nav `<ul>`, after the `ledger` item (`:395-397`):

```jsx
          <li className={`nav-item ${activeTab === 'statement' ? 'active' : ''}`} onClick={() => selectTab('statement')}>
            <Printer size={18} /> Printable Statement
          </li>
```

- [ ] **Step 6: Add the tab branch and the Print control**

In the tab-view region, after the `ledger` branch and inside the same `currentPortfolio ? (<>…</>)` block. The fragment does not create a DOM node, which is what keeps both children direct children of `.main-content`:

```jsx
{activeTab === 'statement' && taxSummary && taxMeta?.portfolioId === selectedId && (
              <>
                <button
                  className="btn btn-primary"
                  data-testid="print-button"
                  onClick={() => window.print()}
                >
                  Print
                </button>
                <PortfolioStatement
                  taxSummary={taxSummary}
                  portfolio={currentPortfolio}
                  fetchedAt={taxMeta.fetchedAt}
                />
              </>
            )}
```

The Print control is **outside** the printed subtree and a sibling of the statement root — that position is what hides it in print. Putting it inside the `<section>` would print a button on the page.

- [ ] **Step 7: Run the test to verify it passes**

Run: `cd frontend && npx vitest run src/test/components/AppStatement.test.tsx`
Expected: PASS, 3 tests.

- [ ] **Step 8: Run the full frontend gate**

Run: `cd frontend && npm run test && npm run lint && npm run typecheck`
Expected: all green. If an existing App test asserted the nav item count, update the count — it is a fixture, not a regression.

- [ ] **Step 9: Commit**

```bash
git add frontend/src/App.tsx frontend/src/test/components/AppStatement.test.tsx
git commit -m "feat(frontend): add the printable statement tab, guarded against a stale summary"
```

---

## Task 5: The local browser smoke

The only layer that sees real print CSS. It is a **local step**, not a CI job — issue #45 owns making it one. It asserts no figure, because figures come from a live provider and a smoke that asserted them would print zeros and still pass.

**Files:**
- Create: `frontend/e2e/statement.smoke.mjs`
- Modify: `frontend/e2e/run.sh` (parameterise the target; export `AUTH_MODE`)
- Modify: `frontend/package.json` (one script)

**Interfaces:**
- Consumes: everything from Tasks 2-4.
- Produces: `npm run e2e:statement`.

- [ ] **Step 1: Fix the harness so it starts at all**

`run.sh` sets no `AUTH_MODE` and there is no `backend_rust/.env`, so the server defaults to OIDC, refuses to launch, and `wait_for backend` burns its whole 420-second budget. This is why the existing smoke does not run today.

In `frontend/e2e/run.sh`, beside the existing `DATABASE_URL` export (`:74-76`):

```sh
export AUTH_MODE=disabled
```

Safe only because `run.sh` builds debug — `AUTH_MODE=disabled` is compiled out of release builds (`backend_rust/src/auth/mod.rs:96-101`). `start.sh:21` already does exactly this.

Then parameterise the target. The hardcoded invocation is at `run.sh:112`:

```sh
node "e2e/${TARGET}"
```

and near the top of the file:

```sh
# ${1:-sell-pnl}, not $1: line 16 is `set -uo pipefail`, so a bare $1 aborts the script
# when it is called with no argument -- which is how npm run e2e:sell-pnl calls it.
TARGET="${1:-sell-pnl.smoke.mjs}"
```

- [ ] **Step 2: Add the npm script**

In `frontend/package.json`, beside the existing `e2e:sell-pnl` (`:16`):

```json
    "e2e:statement": "bash e2e/run.sh statement.smoke.mjs",
```

- [ ] **Step 3: Write the smoke**

Create `frontend/e2e/statement.smoke.mjs`. The seeding block is copied from `sell-pnl.smoke.mjs:35-61` rather than shared — two callers do not make a helper, and #45 can extract one if it inherits a third. **The seeded holdings are denominated in the portfolio's base currency**: a foreign-currency fixture would 500 the tax-summary request and the smoke would print the dashboard instead of a statement, passing while proving nothing.

```js
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
  const [sw, cw] = await statement.evaluate((el) => [el.scrollWidth, el.clientWidth])
  if (sw > cw) fail(`horizontal overflow: scrollWidth ${sw} > clientWidth ${cw}`)

  // A non-empty PDF. No page-count assertion: that needs a PDF parser and
  // frontend/package.json has none. preferCSSPageSize:true so the A4 @page rule is
  // what gets measured -- Playwright's defaults are Letter with false.
  const pdf = await page.pdf({ format: 'A4', preferCSSPageSize: true, printBackground: false })
  if (pdf.length === 0) fail('page.pdf() returned an empty buffer')
  ok(`pdf rendered (${pdf.length} bytes)`)
} catch (err) {
  fail(err.message)
} finally {
  await browser.close()
}

if (failures.length) { console.error(`${failures.length} failure(s)`); process.exit(1) }
console.log('statement smoke OK')
```

**No figure is asserted**, deliberately. The auth alert is **not** asserted either — it renders only from the `auth_error=auth_failed` query param and is not in the DOM under this harness.

- [ ] **Step 4: Run it**

Run: `cd frontend && npm run e2e:statement`
Expected: `statement smoke OK`.

If chromium is missing: `npx playwright install chromium`. On a NixOS workstation the working recipe is `nix-shell -p chromium -p liberation_ttf --run './e2e/run.sh statement.smoke.mjs'` — ask the operator before pulling packages.

- [ ] **Step 5: Verify the existing smoke still runs**

Run: `cd frontend && npm run e2e:sell-pnl`
Expected: passes unchanged. This is the check that `${1:-sell-pnl.smoke.mjs}` did the defaulting correctly; a bare `$1` under `set -u` would have aborted.

- [ ] **Step 6: Commit**

```bash
git add frontend/e2e/statement.smoke.mjs frontend/e2e/run.sh frontend/package.json
git commit -m "test(e2e): add the statement print smoke; make run.sh start and take a target"
```

---

## Task 6: Manual verification and the release note

The print path ships **unverified by CI by design** — there is no browser job (issue #45). That makes the human step a deliverable rather than a formality, and it is the only check of page count, A4-on-Letter behaviour, and whether the thing is actually legible on paper.

**Files:**
- Modify: `docs/auth-testing.md` (append the statement's print walkthrough to §5)
- Create: `docs/release-notes/2026-11-09-portfolio-statement-print.md`

**Interfaces:**
- Consumes: everything above.
- Produces: nothing.

- [ ] **Step 1: Add the manual walkthrough**

Append to `docs/auth-testing.md` §5:

```markdown
### Printing the statement

`npm run dev`, open a portfolio, click **Printable Statement**, then Ctrl-P. Check:

- no sidebar, no nav, no Print button
- dark text on white, and the table's body rules visible (a half-inverted table means
  the `.statement-table td` override in `print.css` is missing)
- one page for a small portfolio
- the three headline figures equal the payload shown in devtools -- **not** "match the
  dashboard". `Dashboard.tsx:39-41` falls back to `metrics.*` when a tax-summary figure
  is falsy, so a coincidental fallback can satisfy an eyeball comparison.
- an empty portfolio prints two lines and no figures

Letter users change the paper size in the print dialog; `@page` says A4 and cannot be
scoped, so printing from any other tab now also gets A4 with 14mm margins.
```

- [ ] **Step 2: Write the release note**

Create `docs/release-notes/2026-11-09-portfolio-statement-print.md` covering: the new tab; that it prints cumulative realized and today's unrealized, with the page saying which is which; the rounding disclosure and why it exists (the CSV is full-precision, the page is 2 dp); the splits caveat; that printing from the other four tabs is unchanged except paper size; and that the print path has no CI check and what was verified instead.

- [ ] **Step 3: Run every gate**

```
cd frontend && npm run test && npm run lint && npm run typecheck
cd backend_rust && cargo test && bash scripts/check_access_gate.sh && cargo build --release
```

Expected: all green. The backend is untouched by this plan; if a backend gate fails, something else landed on the branch.

- [ ] **Step 4: Commit**

```bash
git add docs/auth-testing.md docs/release-notes/2026-11-09-portfolio-statement-print.md
git commit -m "docs: statement print walkthrough and release note"
```

---

## What this plan does not include

- **No CI browser job.** Issue #45. The smoke is a local step; a `continue-on-error` job nobody reads would read as a pass.
- **No price pinning through `historical_prices`.** No API route writes that table, so pinning would mean an out-of-band SQLite write against the harness's own REST-seeding rule — and the smoke asserts no figure, so it buys nothing.
- **No print mode over the existing holdings table.** Rejected as Alternative D in the spec.
- **No print CSS for the other four tabs.** Out of scope, stated in the spec under *Other tabs*.
- **No date parameter.** The statement is today-only by decision; the dated payload is the tax-report document's work.
