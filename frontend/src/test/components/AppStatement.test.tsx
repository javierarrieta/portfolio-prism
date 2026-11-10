import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { http } from 'msw'
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
      // The Print control is hidden by the same rule, so it has to be a SIBLING of the
      // statement root -- a direct child of .main-content too. Inside the <section> it
      // would print as a button on the page; inside a shared wrapper both would vanish.
      expect(screen.getByTestId('print-button').parentElement!.className).toContain('main-content')
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

    // The switcher's "ACTIVE PORTFOLIO" label has no htmlFor and does not wrap the
    // select, so there is no accessible name to query by. getByRole('combobox') is the
    // idiom AppLedger.test.tsx:44 already uses for this same control.
    await user.selectOptions(screen.getByRole('combobox'), 'Retirement Fund')

    await waitFor(() =>
      expect(container.querySelector('[data-print="statement"]')).toBeNull())
    expect(screen.queryByTestId('print-button')).toBeNull()
  })
})
