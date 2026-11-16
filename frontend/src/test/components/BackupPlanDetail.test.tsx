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
    // The headline sentence is the server's totals, spelled from `side.counts` and from
    // nothing else. Deriving it from the 25 listed portfolios — the tempting
    // simplification, since they are already in hand — would print "25 portfolios, 25
    // assets, 400 transactions" two rows above a cap line that says 1204 and 9120, and
    // the two halves of one sentence would contradict each other.
    expect(
      screen.getByText('Will insert 1204 portfolios, 9120 assets, 9120 transactions.'),
    ).toBeInTheDocument();
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
    // Collapsed, and the way out of it on the page: five of the six rows are held back
    // and the control that reveals the sixth says so. A component that truncated the
    // sixth row with no control — what this used to do — fails on the second line.
    expect(screen.getAllByTestId('plan-asset')).toHaveLength(5);
    expect(screen.getByTestId('plan-show-all')).toHaveAttribute('aria-expanded', 'false');
  });

  /** Together with the six-asset case this pins `COLLAPSE_ABOVE` to exactly 5: a threshold
   * of 1-4 collapses these five and puts a toggle on the page, a threshold of 6 or more
   * leaves the six-asset case open. */
  it('leaves a portfolio of exactly five assets open', () => {
    const five = Array.from({ length: 5 }, (_, i) => asset({ symbol: `S${i}` }));
    render(
      <PlanSideView
        title="Will insert"
        side={side({
          shown: [{ name: 'Five', currency: 'USD', assets: 5, transactions: 10, shown: five, more_assets: 0, asset_flags: {}, flags: [] }],
        })}
      />,
    );
    expect(screen.queryByTestId('plan-show-all')).toBeNull();
    expect(screen.getAllByTestId('plan-asset')).toHaveLength(5);
  });

  it('leaves a small portfolio open', () => {
    render(<PlanSideView title="Will insert" side={side()} />);
    expect(screen.queryByTestId('plan-show-all')).toBeNull();
    // No toggle is not the same claim as the rows being on the page. Assert both, or a
    // component that renders the header and drops the rows passes this test.
    expect(screen.getByTestId('plan-portfolio')).toBeInTheDocument();
    expect(screen.getByTestId('plan-asset')).toHaveTextContent('AAA');
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
    const row = screen.getByText(/Wide/).closest('tr');
    // The anchor is the assertion, not the text checks below: it pins the aggregate into
    // the portfolio's own row, the one row a collapse never hides. Were the aggregate
    // instead printed inside an asset row, these checks would be reading a row the
    // collapsed state is holding back — which the last line below is what rules out.
    expect(row).not.toBeNull();
    expect(row?.textContent).toContain('across all assets');
    expect(row?.textContent).toContain('zero-price ×3');
    expect(row?.textContent).toContain('future-dated ×1');
    expect(screen.getAllByTestId('plan-asset')).toHaveLength(5);
  });

  /** The header aggregate counts every asset and the rows count one each, so the header
   * has to say whose numbers they are. */
  it('labels the header aggregate as covering every asset, not the listed ones', () => {
    render(
      <PlanSideView
        title="Will delete"
        side={side({
          shown: [{
            name: 'Wide', currency: 'USD', assets: 318, transactions: 9120,
            shown: [asset()], more_assets: 312,
            asset_flags: { zero_price: 3 }, flags: [],
          }],
        })}
      />,
    );
    expect(screen.getByTestId('plan-portfolio-header').textContent)
      .toContain('across all assets: zero-price ×3');
  });

  /** One code, two units. On a `PlanAsset` `zero_price` counts that asset's zero-price
   * *transactions*; in `asset_flags` it counts flagged *assets*. Nothing pinned the row
   * form, so both printed `zero-price ×N` and the two numbers read as the same kind of
   * thing twice. The header is disambiguated by its prefix; the row has no prefix, so it
   * names its own unit. */
  it('spells the same flag code with the unit each scope counts', () => {
    render(
      <PlanSideView
        title="Will insert"
        side={side({
          shown: [{
            name: 'Wide', currency: 'USD', assets: 318, transactions: 9120,
            shown: [asset({ zero_price: 4, flags: ['zero_price'] })],
            more_assets: 312, asset_flags: { zero_price: 3 }, flags: [],
          }],
        })}
      />,
    );
    // The row: four of this one asset's transactions carry a zero price.
    expect(screen.getByTestId('plan-asset').textContent).toContain('zero-price rows ×4');
    // The header: three of the portfolio's 318 assets carry one. Same code, and the two
    // sentences must not be able to say it in the same words.
    expect(screen.getByTestId('plan-portfolio-header').textContent)
      .toContain('across all assets: zero-price ×3');
    expect(screen.getByTestId('plan-portfolio-header').textContent)
      .not.toContain('zero-price rows');
  });

  it('pluralises the per-asset transaction count', () => {
    render(
      <PlanSideView
        title="Will insert"
        side={side({
          shown: [{
            ...side().shown[0],
            assets: 2, transactions: 4,
            shown: [asset({ symbol: 'AAA', transactions: 1 }), asset({ symbol: 'BBB', transactions: 3 })],
          }],
        })}
      />,
    );
    expect(screen.getByText('1 txn')).toBeInTheDocument();
    expect(screen.getByText('3 txns')).toBeInTheDocument();
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

  /** The portfolio-level case above is not the only place a new code can arrive: the
   * server counts asset flags too, and an unknown one there carries a count. */
  it('renders an unknown asset-flag code with its count', () => {
    render(
      <PlanSideView
        title="Will delete"
        side={side({ shown: [{ ...side().shown[0], asset_flags: { made_up_flag: 2 } }] })}
      />,
    );
    expect(screen.getByText(/made_up_flag ×2/)).toBeInTheDocument();
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

  it('refuses a side whose own two numbers are not numbers', () => {
    expect(isPlanSide({ ...side(), more_portfolios: 'x' })).toBe(false);
    expect(isPlanSide({ ...side(), transactions_in_shown: null })).toBe(false);
  });

  /** `isAsset` runs only through here — it is module-private — so a portfolio whose asset
   * list holds a half-built asset is the only way its branches get exercised. */
  it('refuses a portfolio whose asset list holds a malformed asset', () => {
    expect(isPlanSide({ ...side(), shown: [{ ...side().shown[0], shown: [{ symbol: 'A' }] }] })).toBe(false);
    expect(isPlanSide({ ...side(), shown: [{ ...side().shown[0], shown: [{ ...asset(), transactions: 'x' }] }] })).toBe(false);
  });

  it('refuses an asset-flag tally that is not numbers', () => {
    expect(isPlanSide({ ...side(), shown: [{ ...side().shown[0], asset_flags: { zero_price: 'x' } }] })).toBe(false);
    expect(isPlanSide({ ...side(), shown: [{ ...side().shown[0], asset_flags: null }] })).toBe(false);
  });
});
