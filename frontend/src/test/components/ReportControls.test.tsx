import { describe, it, expect, vi } from 'vitest'
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import ReportControls, { TaxReportTable } from '../../components/ReportControls'
import { currentYearToDate, buildReportQuery, isoDate } from '../../utils/reportWindow'
import { REPORT_FIXTURE } from '../mocks/reportFixture'
import type { ReportErrorBody } from '../../types'

const window2024 = { from: '2024-01-01', to: '2024-06-30' }

const renderControls = (over: Partial<Record<string, unknown>> = {}) => render(
  <ReportControls
    window={window2024}
    onWindowChange={() => {}}
    csvHref=""
    onRetry={() => {}}
    report={null}
    error={null}
    loading={false}
    today={new Date('2026-02-14T00:00:00Z')}
    {...over}
  />,
)

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

  it('does not blame the dates when the server named what is missing', () => {
    // Two different 422s reach the same panel. When the response listed missing symbols or
    // pairs, changing the dates or the strategy cannot help -- the price or rate is not
    // there -- and telling the user otherwise sends them fiddling with a control that is
    // not even on this tab.
    renderControls({
      error: {
        error: 'no price or rate exists at or before the requested date',
        missing_prices: ['ZZZZ'], missing_rates: [],
        missing_prices_total: 1, missing_rates_total: 0, retryable: false,
      },
    })
    expect(screen.getByText(/no price or rate for what is listed/i)).toBeInTheDocument()
    expect(screen.queryByText(/the dates or the strategy have to change/i)).toBeNull()
  })

  it('does blame the window when nothing was listed', () => {
    renderControls({
      error: {
        error: '`to` is in the future', missing_prices: [], missing_rates: [],
        missing_prices_total: 0, missing_rates_total: 0, retryable: false,
      },
    })
    expect(screen.getByText(/the dates or the strategy have to change/i)).toBeInTheDocument()
  })

  it('names the window that was actually applied and the close it used', () => {
    render(<TaxReportTable report={REPORT_FIXTURE} />)
    expect(screen.getByTestId('report-context')).toHaveTextContent('2024-01-01')
    expect(screen.getByTestId('report-context')).toHaveTextContent('2024-06-30')
    expect(screen.getByTestId('row-0-price-date')).toHaveTextContent('2024-06-28')
  })
})

describe('the client clock must agree with the server\'s', () => {
  // The route rejects a `to` after `Utc::now().date_naive()` (analytics.rs:71, :669). Building
  // the client's "today" from LOCAL parts made the default window a future date for every user
  // east of UTC during the first hours of their day, so the tab 422'd on open.
  //
  // *** Read this test's limitation. *** `test/setup.ts` pins TZ=UTC, where local and UTC parts
  // are identical, so this cannot fail on the suite's own clock. It pins the definition -- the
  // value must equal the UTC date -- and would catch a revert to local parts on any host that is
  // not UTC. The fix itself is verified by the server reading UTC, not by this test.
  it('derives the date from UTC parts, not the local calendar', () => {
    // The route rejects a `to` after `Utc::now().date_naive()` (analytics.rs:71, :669). Built
    // from local parts, the default window was a future date for every user east of UTC during
    // the first hours of their day, so the tab 422'd on open and no preset could rescue it.
    //
    // `test/setup.ts` pins TZ=UTC, where local and UTC parts are identical, so the suite's own
    // clock cannot see the difference. Overriding TZ for this one test makes them differ: at
    // UTC+14, noon UTC is already the next local day. Verified to fail when isoDate is reverted
    // to local parts.
    const previous = process.env.TZ
    process.env.TZ = 'Pacific/Kiritimati'
    try {
      const d = new Date('2026-02-14T12:00:00Z')
      expect(d.getDate()).toBe(15)          // the local calendar, which the route would refuse
      expect(isoDate(d)).toBe('2026-02-14') // what the server calls today
    } finally {
      process.env.TZ = previous
    }
  })
})

describe('buildReportQuery', () => {
  it('omits a window side that is empty instead of sending an empty parameter', () => {
    // `from=` is not a date: the route answers 422 "from: expected YYYY-MM-DD". A single-sided
    // window is legal (the server defaults the other side to inception / today), so the empty
    // side must be absent, not blank.
    expect(buildReportQuery(1, { from: '', to: '2024-06-30' }, 'FIFO', 30)).toContain('to=2024-06-30')
    expect(buildReportQuery(1, { from: '', to: '2024-06-30' }, 'FIFO', 30)).not.toContain('from=')
    expect(buildReportQuery(1, { from: '2024-01-01', to: '' }, 'FIFO', 30)).not.toContain('to=')
  })
})

describe('ReportControls guards', () => {
  const render2 = (over: Record<string, unknown> = {}) => render(
    <ReportControls
      window={window2024}
      onWindowChange={() => {}}
      csvHref="/api/x"
      onRetry={() => {}}
      report={null}
      error={null}
      loading={false}
      today={new Date('2026-02-14T00:00:00Z')}
      {...over}
    />,
  )

  it('caps both date inputs at the server-side today', () => {
    // The preset clamp is half the rule; a typed future date is the half a user actually hits.
    render2()
    expect(screen.getByLabelText('To')).toHaveAttribute('max', '2026-02-14')
    expect(screen.getByLabelText('From')).toHaveAttribute('max', '2026-02-14')
  })

  it('says inception when the window had no lower bound', () => {
    // The fixture always has a requested_from, so this branch was unexercised: `?? ''` passed.
    render(<TaxReportTable report={{ ...REPORT_FIXTURE, context: { ...REPORT_FIXTURE.context, requested_from: null } }} />)
    expect(screen.getByTestId('report-context')).toHaveTextContent('inception')
  })
})
