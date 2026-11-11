import { describe, it, expect, vi } from 'vitest'
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import ReportControls, { TaxReportTable } from '../../components/ReportControls'
import { currentYearToDate } from '../../utils/reportWindow'
import { REPORT_FIXTURE } from '../mocks/reportFixture'
import type { ReportErrorBody } from '../../types'

const window2024 = { from: '2024-01-01', to: '2024-06-30' }

describe('currentYearToDate', () => {
  it('defaults to January 1 through today, not December 31', () => {
    // A default ending on December 31 is a future date for most of the year, and the route
    // rejects a future date with a 422 — so the default would be an error, not a default.
    const w = currentYearToDate(new Date('2026-02-14T00:00:00Z'))
    expect(w.from).toBe('2026-01-01')
    expect(w.to).toBe('2026-02-14')
  })
})

describe('ReportControls', () => {
  it('shows the window it was given and links the download at the same window', () => {
    render(
      <ReportControls
        window={window2024}
        onWindowChange={() => {}}
        csvHref="/api/portfolios/1/tax-summary?from=2024-01-01&to=2024-06-30&strategy=FIFO&threshold_days=30&format=csv"
        onRetry={() => {}}
        report={null}
        error={null}
        loading={false}
      />,
    )
    expect(screen.getByLabelText('From')).toHaveValue('2024-01-01')
    expect(screen.getByLabelText('To')).toHaveValue('2024-06-30')
    expect(screen.getByTestId('download-csv')).toHaveAttribute(
      'href',
      '/api/portfolios/1/tax-summary?from=2024-01-01&to=2024-06-30&strategy=FIFO&threshold_days=30&format=csv',
    )
  })

  it('offers a quarter that has not happened yet as unavailable', async () => {
    const onWindowChange = vi.fn()
    render(
      <ReportControls
        window={{ from: '', to: '' }}
        onWindowChange={onWindowChange}
        csvHref=""
        onRetry={() => {}}
        report={null}
        error={null}
        loading={false}
        today={new Date('2026-02-14T00:00:00Z')}
      />,
    )
    // Q1 has begun, Q3 has not. Q1's end is clamped to today: a future `to` is a 422
    // (backend_rust/src/api_routes/analytics.rs:669), so filling the full quarter would make
    // the button's only outcome an error. Q3 is disabled rather than filled with a window
    // that holds nothing, and a preset that silently clamped a future quarter's end would be
    // an invented date.
    await userEvent.click(screen.getByRole('button', { name: 'Q1' }))
    expect(onWindowChange).toHaveBeenCalledWith({ from: '2026-01-01', to: '2026-02-14' })
    expect(screen.getByRole('button', { name: 'Q3' })).toBeDisabled()
  })

  it('reports a 422 by naming the symbols and the count of what was omitted', () => {
    const error: ReportErrorBody = {
      error: 'no price or rate exists at or before the requested date',
      missing_prices: ['AAA', 'BBB'],
      missing_rates: [],
      missing_prices_total: 22,
      missing_rates_total: 0,
      retryable: false,
    }
    render(
      <ReportControls window={window2024} onWindowChange={() => {}} csvHref="" onRetry={() => {}}
        report={null} error={error} loading={false} />,
    )
    expect(screen.getByTestId('report-error')).toHaveTextContent('AAA, BBB')
    expect(screen.getByTestId('report-error')).toHaveTextContent('20 more')
  })

  it('reports a capped rate list as capped, not as the whole list', () => {
    // The rate cap is 10 pairs and truncation has no flag of its own: `length < total` is the
    // only signal, and a capped list shown without its count reads as the complete one.
    const error: ReportErrorBody = {
      error: 'no price or rate exists at or before the requested date',
      missing_prices: [],
      missing_rates: [{ from: 'EUR', to: 'USD' }],
      missing_prices_total: 0,
      missing_rates_total: 4,
      retryable: false,
    }
    render(
      <ReportControls window={window2024} onWindowChange={() => {}} csvHref="" onRetry={() => {}}
        report={null} error={error} loading={false} />,
    )
    expect(screen.getByTestId('report-error')).toHaveTextContent('EUR→USD')
    expect(screen.getByTestId('report-error')).toHaveTextContent('and 3 more')
  })

  it('does not claim omissions when the list is complete', () => {
    // The mirror of the two tests above: a complete list must not carry a count line.
    const error: ReportErrorBody = {
      error: 'no price or rate exists at or before the requested date',
      missing_prices: ['AAA'],
      missing_rates: [],
      missing_prices_total: 1,
      missing_rates_total: 0,
      retryable: false,
    }
    render(
      <ReportControls window={window2024} onWindowChange={() => {}} csvHref="" onRetry={() => {}}
        report={null} error={error} loading={false} />,
    )
    expect(screen.getByTestId('report-error')).toHaveTextContent('AAA')
    expect(screen.getByTestId('report-error')).not.toHaveTextContent('more')
  })

  it('offers no download without a window, because `format=csv` alone is a 422', () => {
    render(
      <ReportControls window={{ from: '', to: '' }} onWindowChange={() => {}} csvHref=""
        onRetry={() => {}} report={null} error={null} loading={false} />,
    )
    expect(screen.queryByTestId('download-csv')).toBeNull()
  })

  it('offers a retry when the deadline trips', async () => {
    const onRetry = vi.fn()
    const error: ReportErrorBody = {
      error: 'the report exceeded its 20 second deadline',
      missing_prices: [], missing_rates: [], missing_prices_total: 0, missing_rates_total: 0,
      retryable: true,
    }
    render(
      <ReportControls window={window2024} onWindowChange={() => {}} csvHref="" onRetry={onRetry}
        report={null} error={error} loading={false} />,
    )
    expect(screen.getByTestId('report-error')).toHaveTextContent('the report exceeded its 20 second deadline')
    await userEvent.click(screen.getByRole('button', { name: /try again/i }))
    expect(onRetry).toHaveBeenCalled()
  })
})

describe('TaxReportTable', () => {
  it('prints the payload totals and never a sum of the rows', () => {
    // Review Focus 2. The rows' `lot_pnl` sums to 1800.00 and the payload's total is
    // 1234.56; the screen must show the server's figure.
    render(<TaxReportTable report={REPORT_FIXTURE} />)
    expect(screen.getByTestId('total-realized-in-window')).toHaveTextContent('$1,234.56')
    expect(screen.getByTestId('total-realized-in-window')).not.toHaveTextContent('1,800.00')
  })

  it('names the window that was actually applied and the close it used', () => {
    render(<TaxReportTable report={REPORT_FIXTURE} />)
    expect(screen.getByTestId('report-context')).toHaveTextContent('2024-01-01')
    expect(screen.getByTestId('report-context')).toHaveTextContent('2024-06-30')
    expect(screen.getByTestId('row-0-price-date')).toHaveTextContent('2024-06-28')
  })
})
