import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { http, HttpResponse } from 'msw'
import App from '../../App'
import { server } from '../mocks/server'
import { REPORT_FIXTURE } from '../mocks/reportFixture'

/** The dated branch of the shared handler, with the query the tab sent. */
const seen: string[] = []

function datedHandler() {
  return http.get('/api/portfolios/:id/tax-summary', ({ request }) => {
    const url = new URL(request.url)
    seen.push(url.search)
    if (!url.searchParams.has('from') && !url.searchParams.has('to')) {
      return HttpResponse.json({ currency: 'USD', total_portfolio_value: 5000, total_realized_pnl: 0, total_unrealized_pnl: 0, assets: [] })
    }
    return HttpResponse.json(REPORT_FIXTURE)
  })
}

describe('the tax report tab', () => {
  beforeEach(() => { seen.length = 0; vi.stubGlobal('alert', vi.fn()); vi.stubGlobal('confirm', vi.fn(() => true)) })
  afterEach(() => vi.unstubAllGlobals())

  it('fetches the dated variant with the current year to date when the tab is opened', async () => {
    server.use(datedHandler())
    render(<App />)
    await userEvent.click(await screen.findByText('Tax Report'))
    await waitFor(() => expect(screen.getByTestId('total-realized-in-window')).toBeInTheDocument())

    const dated = seen.find((q) => q.includes('from=') || q.includes('to='))
    expect(dated).toBeDefined()
    expect(dated).toContain(`to=${new Date().getFullYear()}-`)
    // The tab's own fetch is dated; the undated summary the rest of the app reads is a
    // separate request and stays undated.
    expect(seen.filter((q) => !q.includes('from=') && !q.includes('to=')).length).toBeGreaterThan(0)

    // The download is an anchor with the same window the tab asked for plus `format=csv`:
    // the bytes the route produced, not a client-side re-render of the payload. The API
    // prefix is part of the assertion because `buildReportQuery` returns a server path.
    expect(screen.getByTestId('download-csv').getAttribute('href')).toMatch(
      /^\/api\/portfolios\/1\/tax-summary\?from=\d{4}-01-01&to=\d{4}-\d{2}-\d{2}&strategy=FIFO&threshold_days=30&format=csv$/,
    )
  })

  it('does not render a report that belongs to another portfolio', async () => {
    // Review Focus 3: portfolio 2's report never resolves, so after the switch the state
    // still holds portfolio 1's payload while `selectedId` has moved on. A page with B's
    // numbers under A's heading is the failure this guard exists for.
    //
    // Portfolio 1 is deliberately left to the shared handler in `mocks/handlers.ts`: this is
    // the one test that exercises its dated branch, so deleting that branch -- or letting it
    // fall through to the legacy body -- fails here rather than passing everywhere.
    server.use(http.get('/api/portfolios/2/tax-summary', () => new Promise(() => {})))
    render(<App />)
    await userEvent.click(await screen.findByText('Tax Report'))
    await waitFor(() => expect(screen.getByTestId('total-realized-in-window')).toBeInTheDocument())

    await userEvent.selectOptions(screen.getByRole('combobox'), 'Retirement Fund')
    await waitFor(() => expect(screen.queryByTestId('total-realized-in-window')).toBeNull())
  })

  it('re-fetches the report when the deadline 503 is retried', async () => {
    let dated = 0
    server.use(http.get('/api/portfolios/:id/tax-summary', ({ request }) => {
      const url = new URL(request.url)
      if (!url.searchParams.has('from') && !url.searchParams.has('to')) {
        return HttpResponse.json({ currency: 'USD', total_portfolio_value: 5000, total_realized_pnl: 0, total_unrealized_pnl: 0, assets: [] })
      }
      dated += 1
      if (dated === 1) {
        return HttpResponse.json({
          error: 'the report exceeded its 20 second deadline', missing_prices: [], missing_rates: [],
          missing_prices_total: 0, missing_rates_total: 0, retryable: true,
        }, { status: 503 })
      }
      return HttpResponse.json(REPORT_FIXTURE)
    }))
    render(<App />)
    await userEvent.click(await screen.findByText('Tax Report'))
    await waitFor(() => expect(screen.getByTestId('report-error')).toBeInTheDocument())

    // The retry has to reach the route again: with the same window and strategy, nothing else
    // in the effect's dependencies moves, so an attempt counter that is not a dependency
    // leaves this button inert.
    await userEvent.click(screen.getByRole('button', { name: /try again/i }))
    await waitFor(() => expect(screen.getByTestId('total-realized-in-window')).toBeInTheDocument())
    expect(dated).toBe(2)
  })
})

describe('a failed refetch retires the payload it replaced', () => {
  // Review Critical: `setReportError` alone leaves the previous payload on screen, so a same-key
  // refetch that fails shows numbers under an alert saying they could not be produced. Reachable
  // by a tab round-trip, because `activeTab` is a dependency of the fetch effect.
  it('removes the table when the refetch for the same selection fails', async () => {
    let calls = 0
    server.use(http.get('/api/portfolios/:id/tax-summary', ({ request }) => {
      const url = new URL(request.url)
      if (!url.searchParams.has('from') && !url.searchParams.has('to')) {
        return HttpResponse.json({ currency: 'USD', total_portfolio_value: 5000, total_realized_pnl: 0, total_unrealized_pnl: 0, assets: [] })
      }
      calls += 1
      if (calls === 1) return HttpResponse.json(REPORT_FIXTURE)
      return HttpResponse.json({ error: 'deadline', missing_prices: [], missing_rates: [], missing_prices_total: 0, missing_rates_total: 0, retryable: true }, { status: 503 })
    }))
    render(<App />)
    await userEvent.click(await screen.findByText('Tax Report'))
    await waitFor(() => expect(screen.getByTestId('total-realized-in-window')).toBeInTheDocument())

    await userEvent.click(screen.getByText('Holdings & Tax lots'))
    await userEvent.click(screen.getByText('Tax Report'))

    await waitFor(() => expect(screen.getByTestId('report-error')).toBeInTheDocument())
    expect(screen.queryByTestId('total-realized-in-window')).toBeNull()
    // And the download goes with it: a link to bytes that were never produced.
    expect(screen.queryByTestId('download-csv')).toBeNull()
  })

  it('retires the error when the selection can no longer have produced it', async () => {
    // The mirror of the payload guard, which the controller had already mutation-proved.
    //
    // *** Why this clears the dates rather than switching portfolios. *** Switching portfolios
    // starts a new request, and a new request clears `reportError` at its start, so the keyed
    // guard is never consulted and the test passes with the guard deleted (measured). The guard
    // only decides when NO new request starts: a windowless selection early-returns from the
    // effect, leaving the old keyed body in state.
    server.use(
      http.get('/api/portfolios/:id/tax-summary', ({ request }) =>
        new URL(request.url).searchParams.has('from')
          ? HttpResponse.json({ error: 'no data', missing_prices: ['ZZZZ'], missing_rates: [], missing_prices_total: 1, missing_rates_total: 0, retryable: false }, { status: 422 })
          : HttpResponse.json({ currency: 'USD', total_portfolio_value: 5000, total_realized_pnl: 0, total_unrealized_pnl: 0, assets: [] })),
    )
    render(<App />)
    await userEvent.click(await screen.findByText('Tax Report'))
    await waitFor(() => expect(screen.getByTestId('report-error')).toBeInTheDocument())

    await userEvent.clear(screen.getByLabelText('To'))
    await userEvent.clear(screen.getByLabelText('From'))
    await waitFor(() => expect(screen.queryByTestId('report-error')).toBeNull())
  })

  it('stops showing the spinner when no request is in flight for the selection', async () => {
    // A boolean `reportLoading` stuck at true here: clearing `To` cancels run 1 and starts run
    // 2; clearing `From` then makes the effect early-return, so run 2's `finally` is skipped by
    // the cancelled guard and nothing ever resets the flag. Loading is derived from the
    // in-flight key instead, so a key that no longer matches simply is not loading.
    server.use(http.get('/api/portfolios/:id/tax-summary', () => new Promise(() => {})))
    render(<App />)
    await userEvent.click(await screen.findByText('Tax Report'))
    await waitFor(() => expect(screen.getByText('Loading the report…')).toBeInTheDocument())

    await userEvent.clear(screen.getByLabelText('To'))
    await userEvent.clear(screen.getByLabelText('From'))
    await waitFor(() => expect(screen.queryByText('Loading the report…')).toBeNull())
  })

  it('offers a retry when the request never got an answer', async () => {
    // A dropped connection lands in the generic catch. It used to hardcode `retryable: false`,
    // which told the user their dates were wrong and offered no way to try again -- for a
    // failure with nothing wrong with their dates at all.
    server.use(
      http.get('/api/portfolios/:id/tax-summary', ({ request }) =>
        new URL(request.url).searchParams.has('from')
          ? HttpResponse.error()
          : HttpResponse.json({ currency: 'USD', total_portfolio_value: 5000, total_realized_pnl: 0, total_unrealized_pnl: 0, assets: [] })),
    )
    render(<App />)
    await userEvent.click(await screen.findByText('Tax Report'))
    await waitFor(() => expect(screen.getByTestId('report-error')).toBeInTheDocument())
    expect(screen.getByRole('button', { name: /try again/i })).toBeInTheDocument()
  })

  it('renders a panel, not a blank page, when the error body is not the typed shape', async () => {
    // `body as ReportErrorBody` is compile-time only. A proxy `{}`, Rocket's own envelope, or a
    // truncated body used to reach `error.missing_prices.length` and throw during render, and
    // `main.tsx` has no error boundary -- so the whole app went white.
    server.use(
      http.get('/api/portfolios/:id/tax-summary', ({ request }) =>
        new URL(request.url).searchParams.has('from')
          ? HttpResponse.json({}, { status: 422 })
          : HttpResponse.json({ currency: 'USD', total_portfolio_value: 5000, total_realized_pnl: 0, total_unrealized_pnl: 0, assets: [] })),
    )
    render(<App />)
    await userEvent.click(await screen.findByText('Tax Report'))
    await waitFor(() => expect(screen.getByTestId('report-error')).toBeInTheDocument())
    expect(screen.queryByTestId('total-realized-in-window')).toBeNull()
  })
})
