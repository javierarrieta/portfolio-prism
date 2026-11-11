import { render, screen } from '@testing-library/react';
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import PortfolioStatement from '../../components/PortfolioStatement';
import type { Portfolio, TaxSummary } from '../../types';

const portfolio: Portfolio = {
  id: 1, name: 'Long-Term', currency: 'EUR', assets: [],
};

const asset = {
  symbol: 'AAPL', asset_type: 'stock', current_shares: 10, average_cost: 100,
  current_price: 120, total_cost: 1000, market_value: 1200,
  unrealized_pnl: 200, unrealized_roi: 0.2, realized_pnl: 1234.565,
  tax_lots: [],
};

const summary = (over: Partial<TaxSummary> = {}): TaxSummary => ({
  currency: 'EUR',
  strategy: 'HYBRID',
  threshold_days: 45,
  total_portfolio_value: 10400,
  total_realized_pnl: 1234.565,
  total_unrealized_pnl: -800,
  assets: [asset],
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
    // The unrealized label has no test id of its own; the spec requires this copy
    // literally, so it is pinned by text.
    expect(screen.getByText('Unrealized P&L — as of today')).toBeDefined();
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

  it('omits the threshold text for HYBRID when the payload carries no threshold_days', () => {
    render(
      <PortfolioStatement
        taxSummary={summary({ strategy: 'HYBRID', threshold_days: undefined })}
        portfolio={portfolio} fetchedAt={fetchedAt}
      />
    );
    // Without the `!== undefined` guard the page would print "(threshold undefinedd)".
    expect(screen.getByTestId('run-context').textContent).toBe('Long-Term · EUR · HYBRID');
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

  it('prints a display label for the asset type, and falls through for a type it has never seen', () => {
    render(
      <PortfolioStatement
        taxSummary={summary({ assets: [
          { ...asset, asset_type: 'MUTUAL_FUND' },
          { ...asset, asset_type: 'BOND' },
        ] })}
        portfolio={portfolio} fetchedAt={fetchedAt}
      />
    );
    const types = Array.from(
      document.querySelectorAll('[data-testid="holdings"] tbody td:nth-child(2)')
    ).map((td) => td.textContent);
    // MUTUAL_FUND is the widest unbreakable token the enum can carry and an internal label, not
    // financial data, so the printed document shortens it. An unmapped value must still print --
    // a blank Type cell would lose information silently.
    expect(types).toEqual(['Fund', 'BOND']);
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
