import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { PlanSideView, isPlanSide, type PlanSide } from '../../components/BackupPlanDetail';

const asset = (over: Record<string, unknown> = {}) => ({
  symbol: 'AAA', name: 'Alpha', currency: 'USD', transactions: 2,
  first_date: '2026-01-05', last_date: '2026-02-05', zero_price: 0, flags: [], ...over,
});

const side = (over: Record<string, unknown> = {}): PlanSide => ({
  counts: { portfolios: 1, assets: 1, transactions: 2 },
  shown: [{
    name: 'Alpha Fund', currency: 'USD', assets: 1, transactions: 2,
    shown: [asset()], more_assets: 0, asset_flags: {}, flags: [],
  }],
  more_portfolios: 0,
  transactions_in_shown: 2,
  ...over,
} as PlanSide);

describe('PlanSideView', () => {
  it('names the portfolio and its counts', () => {
    render(<PlanSideView title="Will insert" side={side()} />);
    expect(screen.getByText(/Will insert 1 portfolio, 1 asset, 2 transactions/)).toBeInTheDocument();
    expect(screen.getByText(/Alpha Fund/)).toBeInTheDocument();
    expect(screen.getByText(/AAA/)).toBeInTheDocument();
  });

  it('says what is not listed, without changing what is counted', () => {
    // 25 shown against 1,204 counted. The cap line has to name both, and the counts
    // line has to stay the server's total rather than the sample's.
    const shown25 = Array.from({ length: 25 }, (_, i) => ({ ...side().shown[0], name: `P${i}` }));
    render(
      <PlanSideView
        title="Will insert"
        side={side({
          counts: { portfolios: 1204, assets: 9120, transactions: 9120 },
          shown: shown25,
          more_portfolios: 1179,
          transactions_in_shown: 400,
        })}
      />,
    );
    expect(screen.getByText(/showing 25 of 1204 portfolios/)).toBeInTheDocument();
    expect(screen.getByText(/the portfolios shown hold 400 of 9120 transactions/)).toBeInTheDocument();
  });

  it('collapses a portfolio with more than five assets', () => {
    const many = Array.from({ length: 6 }, (_, i) => asset({ symbol: `S${i}` }));
    render(
      <PlanSideView
        title="Will insert"
        side={side({
          shown: [{ name: 'Wide', currency: 'USD', assets: 6, transactions: 12, shown: many, more_assets: 0, asset_flags: {}, flags: [] }],
        })}
      />,
    );
    const block = document.querySelector('details[data-testid="plan-portfolio"]');
    expect(block).not.toBeNull();
    expect(block?.hasAttribute('open')).toBe(false);
  });

  it('leaves a small portfolio open', () => {
    render(<PlanSideView title="Will insert" side={side()} />);
    expect(document.querySelector('details[data-testid="plan-portfolio"]')).toBeNull();
  });

  /** The collapse rule would otherwise lose this: a flag must be visible without
   * expanding anything, or it was never raised. */
  it('shows aggregated asset flags on a collapsed header', () => {
    const many = Array.from({ length: 6 }, (_, i) => asset({ symbol: `S${i}` }));
    render(
      <PlanSideView
        title="Will delete"
        side={side({
          shown: [{
            name: 'Wide', currency: 'USD', assets: 318, transactions: 9120, shown: many,
            more_assets: 312, asset_flags: { zero_price: 3, future_dated: 1 }, flags: [],
          }],
        })}
      />,
    );
    const summary = screen.getByText(/Wide/).closest('summary') ?? document.body;
    expect(summary.textContent).toContain('zero-price ×3');
    expect(summary.textContent).toContain('future-dated ×1');
  });

  it('renders a flag code it does not know instead of throwing', () => {
    render(
      <PlanSideView
        title="Will insert"
        side={side({ shown: [{ ...side().shown[0], flags: ['made_up_flag'] }] })}
      />,
    );
    expect(screen.getByText(/made_up_flag/)).toBeInTheDocument();
  });

  it('marks a portfolio with no assets', () => {
    render(
      <PlanSideView
        title="Will insert"
        side={side({
          counts: { portfolios: 1, assets: 0, transactions: 0 },
          shown: [{ name: 'Hollow', currency: 'USD', assets: 0, transactions: 0, shown: [], more_assets: 0, asset_flags: {}, flags: ['no_assets'] }],
          transactions_in_shown: 0,
        })}
      />,
    );
    expect(screen.getByText(/no assets/)).toBeInTheDocument();
  });
});

describe('isPlanSide', () => {
  it('accepts a real side', () => {
    expect(isPlanSide(side())).toBe(true);
  });

  it('refuses a body that is not a side', () => {
    expect(isPlanSide(null)).toBe(false);
    expect(isPlanSide({})).toBe(false);
    expect(isPlanSide({ ...side(), shown: 'nope' })).toBe(false);
    expect(isPlanSide({ ...side(), shown: [{ name: 'x' }] })).toBe(false);
    expect(isPlanSide({ ...side(), counts: { portfolios: 1 } })).toBe(false);
  });
});
