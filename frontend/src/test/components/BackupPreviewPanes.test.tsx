import { render, screen, waitFor, within, fireEvent } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import BackupPanel from '../../components/BackupPanel'

/** The two panes of the restore preview: what each one is allowed to contain, and what
 * folding one of them is allowed to hide.
 *
 * `BackupPreviewTables.test.tsx` pins the tables inside a pane and
 * `BackupPlanDetail.test.tsx` pins the plan contract. This file pins the two things on
 * top of them: that `copy` mode renders one pane rather than a pane of five words, and
 * that a pane can be folded without the fold reaching anywhere it must not — not the
 * counts sentence, not the per-portfolio expansion, not the Restore button.
 *
 * Fixtures are synthetic throughout: `A0`, `OLD0`, `Alpha Fund`, `Old Fund`. */

const asset = (over: Record<string, unknown> = {}) => ({
  symbol: 'AAA', name: 'Alpha Index', currency: 'USD', transactions: 2,
  first_date: '2024-01-15', last_date: '2024-06-20', zero_price: 0, flags: [], ...over,
});

/** A side listing one portfolio. `assets` is how many that portfolio lists, so 8 is what
 * puts the per-portfolio "show all N listed assets" control on the page. */
const onePortfolioSide = (name: string, symbol: string, assets: number) => ({
  counts: { portfolios: 1, assets, transactions: 9120 },
  shown: [{
    name, currency: 'USD', assets, transactions: 9120,
    shown: Array.from({ length: assets }, (_, i) => asset({ symbol: `${symbol}${i}` })),
    more_assets: 0, asset_flags: {}, flags: [],
  }],
  more_portfolios: 0,
  transactions_in_shown: 9120,
});

const planBody = (mode: string) => ({
  schema_version: 1,
  mode,
  delete: onePortfolioSide('Old Fund', 'OLD', 1),
  insert: onePortfolioSide('Alpha Fund', 'A', 8),
  prices: 0,
  warnings: [],
})

const bundle = () => new File([JSON.stringify({
  format: 'portfolio-prism-backup', schema_version: 1,
  created_at: '2026-11-13T00:00:00Z',
  portfolios: [{ ref: 'p1', name: 'Alpha Fund', description: null, currency: 'USD',
    base_currency: 'USD', assets: [] }],
})], 'bundle.json', { type: 'application/json' })

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } })
}

const stubPreview = () => vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL) => {
  const url = String(input)
  if (url.includes('/backup/preview')) {
    return jsonResponse(planBody(url.includes('mode=replace') ? 'replace' : 'copy'))
  }
  return jsonResponse({}, 404)
}))

/** Uploads the bundle in `mode` and waits for the plan to land, so a test below starts
 * from a preview on screen rather than one still in flight. */
async function previewIn(mode: 'copy' | 'replace') {
  render(<BackupPanel portfolios={[{ id: 1, name: 'Main' }]} />)
  if (mode === 'replace') {
    await userEvent.click(screen.getByRole('radio', { name: /Replace/ }))
  }
  await userEvent.upload(screen.getByTestId('bundle-file'), bundle())
  await waitFor(() => expect(screen.getByTestId('preview-summary')).toBeInTheDocument())
}

const fold = (testid: string) =>
  fireEvent.click(within(screen.getByTestId(testid)).getByTestId('plan-side-toggle'))

describe('copy mode has one pane', () => {
  beforeEach(stubPreview)
  afterEach(() => vi.unstubAllGlobals())

  it('renders one panel, and no delete pane to compare it against', async () => {
    await previewIn('copy')

    // One panel, not two halves of a row. The delete half has no listing in `copy`, so
    // a second panel there is a frame around nothing — and a frame is a track of the
    // grid, which is half the width taken from the one side that has anything to say.
    expect(document.querySelectorAll('.plan-side-panel')).toHaveLength(1)
    expect(screen.getAllByTestId('plan-side')).toHaveLength(1)
    expect(screen.getByTestId('preview-insert')).toHaveClass('plan-side-panel')

    // The grid itself, not just the count of panels: one child, and the modifier that
    // tells `.plan-sides` to stop reserving a track for the side that has no content.
    const grid = screen.getByTestId('preview-insert').closest('.plan-sides')!
    expect(grid).toHaveClass('plan-sides--one')
    expect(grid.children).toHaveLength(1)
  })

  it('still says `Will delete nothing.`, as a note above the panel rather than as one', async () => {
    await previewIn('copy')

    const note = screen.getByTestId('preview-delete')
    expect(note).toHaveTextContent('Will delete nothing.')
    // Still the delete half's own element — but a line above the panel, not a second
    // panel, and not a track of the grid beside it. Dropping the sentence along with the
    // pane is the failure the first line refuses; the last two refuse the other half.
    expect(note).not.toHaveClass('plan-side-panel')
    expect(note).toHaveClass('plan-side-none')
    expect(note.closest('.plan-sides')).toBeNull()
  })

  it('gives the single copy-mode pane a working header toggle', async () => {
    await previewIn('copy')

    const toggle = screen.getByTestId('plan-side-toggle')
    expect(toggle).toHaveAttribute('aria-expanded', 'true')
    fireEvent.click(toggle)
    expect(screen.getByTestId('plan-side-toggle')).toHaveAttribute('aria-expanded', 'false')
    expect(screen.getByTestId('plan-portfolio-table')).not.toBeVisible()
  })
})

describe('folding a preview pane', () => {
  beforeEach(() => vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL) => {
    const url = String(input)
    if (url.includes('/backup/preview')) {
      return jsonResponse(planBody(url.includes('mode=replace') ? 'replace' : 'copy'))
    }
    if (url.includes('/backup/restore')) {
      return jsonResponse({ inserted: { portfolios: 1, assets: 2, transactions: 3 },
        portfolio_ids: [7], prices_inserted: 0 })
    }
    return jsonResponse({}, 404)
  })))
  afterEach(() => vi.unstubAllGlobals())

  it('starts both panes expanded, so nothing a destructive flow depends on starts hidden', async () => {
    await previewIn('replace')

    for (const testid of ['preview-delete', 'preview-insert']) {
      const pane = screen.getByTestId(testid)
      expect(within(pane).getByTestId('plan-side-toggle')).toHaveAttribute('aria-expanded', 'true')
      expect(within(pane).getByTestId('plan-portfolio-table')).toBeVisible()
    }
  })

  it('hides the table when a pane is folded, and keeps its title and its counts sentence', async () => {
    await previewIn('replace')
    const pane = screen.getByTestId('preview-delete')
    // The counts sentence is the pane's totals — 1 portfolio, 1 asset, 9120
    // transactions. Read it before folding so the assertion below names that sentence
    // rather than whatever text happens to survive the fold.
    const counts = within(pane).getByText('Will delete 1 portfolio, 1 asset, 9120 transactions.')
    expect(counts).toBeVisible()
    expect(within(pane).getByText('Old Fund')).toBeVisible()

    fold('preview-delete')

    // What the fold is for: the listing goes.
    expect(within(pane).getByTestId('plan-portfolio-table')).not.toBeVisible()
    expect(within(pane).getByTestId('plan-portfolio')).not.toBeVisible()
    expect(within(pane).getByTestId('plan-side-toggle')).toHaveAttribute('aria-expanded', 'false')
    // What the fold is not allowed to take. A folded pane that hides its own totals
    // cannot be read against the pane left open, and reading them against each other is
    // the only reason anyone folds one.
    expect(counts).toBeVisible()
    expect(within(pane).getByText('Old Fund')).not.toBeVisible()
    // And only that pane folded: the other still shows its listing.
    expect(within(screen.getByTestId('preview-insert')).getByTestId('plan-portfolio-table'))
      .toBeVisible()
  })

  it('reaches the toggle by keyboard, since it is a real button and not a click target', async () => {
    await previewIn('replace')
    const toggle = within(screen.getByTestId('preview-delete')).getByTestId('plan-side-toggle')

    // Tab from the top of the card until the toggle is reached, rather than aiming at it:
    // a header you can only fold with a mouse is a header that hides its listing from the
    // keyboard. The bound is so that a card growing another control fails with a count
    // instead of hopping forever.
    let hops = 0
    while (document.activeElement !== toggle && hops < 12) {
      await userEvent.tab()
      hops += 1
    }
    expect(document.activeElement).toBe(toggle)
    expect(toggle).toHaveAttribute('aria-expanded', 'true')

    await userEvent.keyboard('{Enter}')
    expect(within(screen.getByTestId('preview-delete')).getByTestId('plan-side-toggle'))
      .toHaveAttribute('aria-expanded', 'false')
    expect(within(screen.getByTestId('preview-delete')).getByTestId('plan-portfolio-table'))
      .not.toBeVisible()
  })

  it('keeps a per-portfolio expansion the reader opened across a fold and its undo', async () => {
    await previewIn('replace')
    const pane = screen.getByTestId('preview-insert')
    const rows = () => within(pane).queryAllByTestId('plan-asset')

    // The per-portfolio control is a different control at a different level: it lists
    // 5 of 8 rows until asked, and it sits inside the pane, not in its header.
    expect(rows()).toHaveLength(5)
    fireEvent.click(within(pane).getByTestId('plan-show-all'))
    expect(within(pane).getByTestId('plan-show-all')).toHaveAttribute('aria-expanded', 'true')
    expect(rows()).toHaveLength(8)

    fold('preview-insert')
    // Folded: still the same eight rows, just not on show. A fold that unmounted the
    // body would drop them out of the document here rather than hide them.
    expect(rows()).toHaveLength(8)
    expect(rows()[0]).not.toBeVisible()
    expect(rows()[7]).not.toBeVisible()

    fold('preview-insert')
    // Unfolded: the same content in the same state. Unmounting on the fold would
    // remount every portfolio collapsed, and the reader would find three rows they never
    // asked to hide back under a control reading `show all 8 listed assets`.
    expect(rows()).toHaveLength(8)
    expect(within(pane).getByTestId('plan-show-all')).toHaveAttribute('aria-expanded', 'true')
    expect(rows()[0]).toBeVisible()
    expect(rows()[7]).toBeVisible()
  })

  it('leaves Restore disabled folded and unfolded alike, before REPLACE is typed', async () => {
    await previewIn('replace')
    expect(screen.getByTestId('restore-button')).toBeDisabled()

    fold('preview-delete')
    expect(screen.getByTestId('restore-button')).toBeDisabled()

    fold('preview-insert')
    expect(screen.getByTestId('restore-button')).toBeDisabled()

    fold('preview-delete')
    fold('preview-insert')
    expect(screen.getByTestId('restore-button')).toBeDisabled()
  })

  it('leaves Restore enabled folded and unfolded alike, once REPLACE is typed', async () => {
    await previewIn('replace')
    await userEvent.type(screen.getByTestId('replace-confirm'), 'REPLACE')
    await waitFor(() => expect(screen.getByTestId('restore-button')).toBeEnabled())

    fold('preview-delete')
    // Folding the delete half must not retire the confirmation typed against it, and
    // must not arm the write either: the button reads the plan, not how it is displayed.
    expect(screen.getByTestId('restore-button')).toBeEnabled()
    expect(screen.getByTestId('replace-confirm')).toHaveValue('REPLACE')

    fold('preview-insert')
    expect(screen.getByTestId('restore-button')).toBeEnabled()

    fold('preview-delete')
    fold('preview-insert')
    expect(screen.getByTestId('restore-button')).toBeEnabled()
  })

  it('leaves Restore enabled folded and unfolded alike in copy mode, where nothing needs REPLACE', async () => {
    await previewIn('copy')
    expect(screen.getByTestId('restore-button')).toBeEnabled()

    fireEvent.click(screen.getByTestId('plan-side-toggle'))
    expect(screen.getByTestId('restore-button')).toBeEnabled()

    fireEvent.click(screen.getByTestId('plan-side-toggle'))
    expect(screen.getByTestId('restore-button')).toBeEnabled()
  })
})
