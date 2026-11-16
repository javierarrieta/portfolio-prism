import { render, screen, waitFor, within, fireEvent } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import BackupPanel from '../../components/BackupPanel'
import { PlanSideView, type PlanSide } from '../../components/BackupPlanDetail'

/** The preview as two tables you can compare, rather than two stacks of prose.
 * `BackupPlanDetail.test.tsx` pins the plan contract — the counts sentence, the caps,
 * the flag scopes — and this file pins the layout that grew on top of it: one table per
 * half with named columns, a portfolio row above the asset rows it owns, flags as
 * badges, and a collapse that is a control rather than a truncation.
 *
 * Fixtures are synthetic throughout: `AAA`, `Alpha Fund`, `2024-01-15`. */

const asset = (over: Record<string, unknown> = {}) => ({
  symbol: 'AAA', name: 'Alpha Index', currency: 'USD', transactions: 2,
  first_date: '2024-01-15', last_date: '2024-06-20', zero_price: 0, flags: [], ...over,
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

const PORTFOLIO_COLUMNS = ['Portfolio', 'Currency', 'Assets', 'Transactions', 'Flags'];
const ASSET_COLUMNS = ['Symbol', 'Name', 'Currency', 'Txns', 'First', 'Last', 'Flags'];

describe('the preview as two tables', () => {
  function jsonResponse(body: unknown, status = 200): Response {
    return new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } })
  }

  const bundle = () => new File([JSON.stringify({
    format: 'portfolio-prism-backup', schema_version: 1,
    created_at: '2026-11-13T00:00:00Z',
    portfolios: [{ ref: 'p1', name: 'Alpha Fund', description: null, currency: 'USD',
      base_currency: 'USD', assets: [] }],
  })], 'bundle.json', { type: 'application/json' })

  /** A plan whose halves each list one portfolio with one asset, so both halves have a
   * table to render. `replace` is the mode that gives the delete half a listing at all —
   * in `copy` it says `Will delete nothing.` and has nothing to compare against. */
  const planBody = (mode: string) => ({
    schema_version: 1,
    mode,
    delete: {
      counts: { portfolios: 1, assets: 1, transactions: 6 },
      shown: [{
        name: 'Old Fund', currency: 'USD', assets: 1, transactions: 6,
        shown: [asset({ symbol: 'OLD', name: 'Old Holding', transactions: 6 })],
        more_assets: 0, asset_flags: {}, flags: [],
      }],
      more_portfolios: 0, transactions_in_shown: 6,
    },
    insert: {
      counts: { portfolios: 1, assets: 1, transactions: 2 },
      shown: [{
        name: 'Alpha Fund', currency: 'USD', assets: 1, transactions: 2,
        shown: [asset()], more_assets: 0, asset_flags: {}, flags: [],
      }],
      more_portfolios: 0, transactions_in_shown: 2,
    },
    prices: 0, warnings: [],
  })

  beforeEach(() => {
    vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input)
      if (url.includes('/backup/preview')) {
        const mode = url.includes('mode=replace') ? 'replace' : 'copy'
        return jsonResponse(planBody(mode))
      }
      return jsonResponse({}, 404)
    }))
  })
  afterEach(() => vi.unstubAllGlobals())

  it('gives each half its own table, with the columns named and the side named', async () => {
    render(<BackupPanel portfolios={[{ id: 1, name: 'Main' }]} />)
    await userEvent.click(screen.getByRole('radio', { name: /Replace/ }))
    await userEvent.upload(screen.getByTestId('bundle-file'), bundle())
    await waitFor(() => expect(screen.getByTestId('preview-insert')).toBeInTheDocument())

    for (const [testid, title, name] of [
      ['preview-delete', 'Will delete', 'Old Fund'],
      ['preview-insert', 'Will insert', 'Alpha Fund'],
    ] as const) {
      const half = screen.getByTestId(testid)
      // The two halves are the two panels of one comparison, so each carries its own
      // table rather than one shared list.
      expect(half).toHaveClass('plan-side-panel')
      const head = within(half).getByTestId('plan-portfolio-head')
      for (const column of PORTFOLIO_COLUMNS) {
        expect(within(head).getByRole('columnheader', { name: column })).toBeInTheDocument()
      }
      const assetHead = within(half).getByTestId('plan-asset-head')
      for (const column of ASSET_COLUMNS) {
        expect(within(assetHead).getByRole('columnheader', { name: column })).toBeInTheDocument()
      }
      // Which side a reader has landed in, from the table itself rather than from where
      // it happens to sit on the page.
      expect(within(half).getByTestId('plan-portfolio-table').querySelector('caption')?.textContent)
        .toContain(title)
      // And the half really is that side's: the delete table lists the database's
      // portfolio, not the bundle's.
      expect(half).toHaveTextContent(name)
    }
  })

  it('names the side and the portfolio on each asset table', async () => {
    render(<BackupPanel portfolios={[{ id: 1, name: 'Main' }]} />)
    await userEvent.click(screen.getByRole('radio', { name: /Replace/ }))
    await userEvent.upload(screen.getByTestId('bundle-file'), bundle())
    await waitFor(() => expect(screen.getByTestId('preview-insert')).toBeInTheDocument())

    // The portfolio table carries its side in a caption; the asset table is one level
    // down and carries its own name, because a reader who tabs into it has lost the
    // heading above it. Both halves are asserted so the name cannot be a constant: an
    // asset table that said only "assets in Alpha Fund" would be ambiguous between the
    // two tables on the page, and one that dropped the portfolio name would not say
    // which portfolio's assets these are.
    expect(screen.getByRole('table', { name: 'Will delete — assets in Old Fund' })).toBeInTheDocument()
    expect(screen.getByRole('table', { name: 'Will insert — assets in Alpha Fund' })).toBeInTheDocument()
  })

  it('shows a portfolio row with its counts and a nested asset row with its symbol', () => {
    render(<PlanSideView title="Will insert" side={side()} />)

    const row = screen.getByTestId('plan-portfolio-header')
    expect(within(row).getByRole('rowheader', { name: 'Alpha Fund' })).toBeInTheDocument()
    expect(within(row).getByRole('cell', { name: 'USD' })).toBeInTheDocument()
    expect(within(row).getByRole('cell', { name: '1 asset' })).toBeInTheDocument()
    expect(within(row).getByRole('cell', { name: '2 transactions' })).toBeInTheDocument()

    const assetRow = screen.getByTestId('plan-asset')
    expect(within(assetRow).getByRole('rowheader', { name: 'AAA' })).toBeInTheDocument()
    expect(within(assetRow).getByRole('cell', { name: '2024-01-15' })).toBeInTheDocument()
    expect(within(assetRow).getByRole('cell', { name: '2024-06-20' })).toBeInTheDocument()
    // Nested, not merely adjacent: the asset row sits inside its portfolio's own row
    // group, so a second portfolio cannot borrow this one's rows.
    expect(screen.getByTestId('plan-portfolio').contains(assetRow)).toBe(true)
  })

  it('renders each flag as a badge, with the text unchanged', () => {
    render(
      <PlanSideView
        title="Will insert"
        side={side({
          shown: [{
            ...side().shown[0],
            shown: [asset({ zero_price: 4, flags: ['zero_price'] })],
            flags: ['no_transactions'],
          }],
        })}
      />,
    );
    const portfolioFlag = screen.getByText('no transactions');
    expect(portfolioFlag).toHaveClass('badge', 'plan-flag');
    const assetFlag = screen.getByText('zero-price rows ×4');
    expect(assetFlag).toHaveClass('badge', 'plan-flag');
  });

  it('gives a portfolio with no assets a row saying so, not an empty table body', () => {
    render(
      <PlanSideView
        title="Will insert"
        side={side({
          counts: { portfolios: 1, assets: 0, transactions: 0 },
          shown: [{
            name: 'Hollow', currency: 'USD', assets: 0, transactions: 0,
            shown: [], more_assets: 0, asset_flags: {}, flags: [],
          }],
          transactions_in_shown: 0,
        })}
      />,
    );
    expect(screen.getByTestId('plan-no-assets')).toHaveTextContent('No assets in this portfolio.');
    expect(screen.queryAllByTestId('plan-asset')).toHaveLength(0);
    // The headers are still there, which is what makes the row an answer about an empty
    // portfolio rather than a table that failed to fill in.
    expect(screen.getByTestId('plan-asset-head')).toBeInTheDocument();
  });
});

describe('the collapse toggle', () => {
  const asset = (over: Record<string, unknown> = {}) => ({
    symbol: 'AAA', name: 'Alpha Index', currency: 'USD', transactions: 2,
    first_date: '2024-01-15', last_date: '2024-06-20', zero_price: 0, flags: [], ...over,
  });

  const wide = (shown: number, moreAssets: number): PlanSide => ({
    counts: { portfolios: 1, assets: shown + moreAssets, transactions: 9120 },
    shown: [{
      name: 'Alpha Fund', currency: 'USD', assets: shown + moreAssets, transactions: 9120,
      shown: Array.from({ length: shown }, (_, i) => asset({ symbol: `S${i}` })),
      more_assets: moreAssets, asset_flags: {}, flags: [],
    }],
    more_portfolios: 0,
    transactions_in_shown: 9120,
  });

  it('starts collapsed above the threshold, expands in place, and keeps the server cap beside it', () => {
    render(<PlanSideView title="Will insert" side={wide(8, 312)} />);

    // Collapsed to five of the eight rows the response carried, with the way out on page.
    expect(screen.getAllByTestId('plan-asset')).toHaveLength(5);
    const toggle = screen.getByTestId('plan-show-all');
    expect(toggle).toHaveAttribute('aria-expanded', 'false');
    // "listed", because the count is the server's cap on the listing, not the
    // portfolio's holding: at the cap this reads `show all 50 listed assets` directly
    // above `and 312 more assets`, and the two numbers are then about two different
    // totals. Without "listed" the control claims the 50 are everything there is.
    expect(toggle).toHaveTextContent('show all 8 listed assets');
    // The server's cap is a different fact — those 312 rows never arrived — and it is on
    // the page while collapsed, so nothing about the collapsed state hides it.
    expect(screen.getByText('and 312 more assets')).toBeInTheDocument();

    fireEvent.click(toggle);

    // Expanded in place: the same table, three more rows, nothing navigated away to.
    expect(screen.getAllByTestId('plan-asset')).toHaveLength(8);
    expect(screen.getByTestId('plan-show-all')).toHaveAttribute('aria-expanded', 'true');
    // And expanding the local list does not spend the cap: the two mechanisms stay
    // separate, so the note outlives the toggle rather than being folded into it.
    expect(screen.getByText('and 312 more assets')).toBeInTheDocument();
  });
});
