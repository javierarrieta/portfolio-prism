import { describe, it, expect, vi, afterEach } from 'vitest'
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import PortfolioDetail from '../../components/PortfolioDetail'
import type { Portfolio, TaxSummary } from '../../types'

const asset = {
  id: 1,
  portfolio_id: 1,
  symbol: 'AAPL',
  name: 'Apple Inc.',
  asset_type: 'STOCK' as const,
  sector: 'Technology',
  currency: 'USD',
  isin: null,
  transactions: [],
}

const portfolio = (splitsLastSyncedAt?: string | null): Portfolio => ({
  id: 1,
  name: 'Test Portfolio',
  description: undefined,
  currency: 'USD',
  assets: [asset],
  splits_last_synced_at: splitsLastSyncedAt,
})

const emptyTaxSummary: TaxSummary = {
  assets: [],
  currency: 'USD',
  total_portfolio_value: 0,
  total_realized_pnl: 0,
  total_unrealized_pnl: 0,
  total_market_value: 0,
}

const noop = async () => {}

function renderDetail(portfolioValue: Portfolio, onFetchPortfolioData = noop) {
  return render(
    <PortfolioDetail
      onFetchPortfolioData={onFetchPortfolioData}
      portfolio={portfolioValue}
      taxSummary={emptyTaxSummary}
      onAddAsset={noop}
      onDeleteAsset={noop}
      onDeletePortfolio={noop}
      onAddTransaction={noop}
      strategy="FIFO"
      setStrategy={() => {}}
      thresholdDays={30}
      setThresholdDays={() => {}}
    />
  )
}

function jsonResponse(status: number, body: unknown) {
  return {
    ok: status >= 200 && status < 300,
    status,
    json: async () => body,
  } as Response
}

afterEach(() => {
  vi.restoreAllMocks()
})

describe('PortfolioDetail split sync', () => {
  it('names the last sync date when the portfolio is covered', () => {
    renderDetail(portfolio('2026-11-01'))
    expect(screen.getByText('Splits last synced 2026-11-01')).toBeVisible()
  })

  // The distinction matters: a bare date for a portfolio where one asset has never
  // been synced would read as "recently checked" while a holding sat unsynced.
  it('says not-yet-synced rather than an age when it is not covered', () => {
    renderDetail(portfolio(null))
    expect(screen.getByText('Splits not yet synced for every asset')).toBeVisible()
    expect(screen.queryByText(/Splits last synced/)).not.toBeInTheDocument()
  })

  it('posts to the sync endpoint and reports what was recorded', async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      jsonResponse(200, { assets_checked: 3, splits_inserted: 2, failures: [] })
    )
    vi.stubGlobal('fetch', fetchMock)
    const refreshed = vi.fn(async () => {})

    renderDetail(portfolio(null), refreshed)
    fireEvent.click(screen.getByText('Sync splits'))

    await waitFor(() => expect(fetchMock).toHaveBeenCalled())
    const [url, init] = fetchMock.mock.calls[0]
    expect(String(url)).toContain('/portfolios/1/sync-splits')
    expect((init as RequestInit).method).toBe('POST')

    await waitFor(() =>
      expect(screen.getByRole('status').textContent).toContain('Recorded 2 splits')
    )
    // A new split changes cost basis across the panel, so the page re-reads.
    await waitFor(() => expect(refreshed).toHaveBeenCalled())
  })

  it('says so when there was nothing new', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(jsonResponse(200, { assets_checked: 3, splits_inserted: 0, failures: [] }))
    )
    const refreshed = vi.fn(async () => {})

    renderDetail(portfolio('2026-11-01'), refreshed)
    fireEvent.click(screen.getByText('Sync splits'))

    await waitFor(() => expect(screen.getByRole('status').textContent).toContain('No new splits'))
    expect(refreshed).not.toHaveBeenCalled()
  })

  it('surfaces a provider outage instead of rendering it as success', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(
        jsonResponse(503, { error: 'pricing provider unavailable: all 3 asset(s) failed to sync' })
      )
    )

    renderDetail(portfolio(null))
    fireEvent.click(screen.getByText('Sync splits'))

    await waitFor(() =>
      expect(screen.getByRole('status').textContent).toContain('pricing provider unavailable')
    )
  })

  it('names the assets that failed when the sync was partial', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(
      jsonResponse(200, {
        assets_checked: 3,
        splits_inserted: 1,
        failures: [{ asset_id: 7, symbol: 'XYZ', error: 'timeout' }],
      })
    ))

    renderDetail(portfolio(null))
    fireEvent.click(screen.getByText('Sync splits'))

    await waitFor(() => {
      const text = screen.getByRole('status').textContent ?? ''
      expect(text).toContain('Recorded 1 split')
      expect(text).toContain('1 asset(s) could not be synced')
    })
  })
})
