import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import App from '../../App'
import { server } from '../mocks/server'
import { http, HttpResponse } from 'msw'

describe('App ledger → sell P&L', () => {
  beforeEach(() => {
    vi.stubGlobal('alert', vi.fn())
    vi.stubGlobal('confirm', vi.fn(() => true))
  })
  afterEach(() => vi.unstubAllGlobals())

  it('opens the breakdown when a SELL row is clicked', async () => {
    const user = userEvent.setup()
    server.use(http.get('/api/portfolios/1/transactions', () => HttpResponse.json([
      { id: 10, asset_id: 1, type: 'SELL', quantity: '80.0', price: '130.0', fee: '0.0', date: '2024-12-01T00:00:00Z' },
    ])))
    render(<App />)

    await waitFor(() => expect(screen.getByText('Transaction Ledger')).toBeInTheDocument())
    await user.click(screen.getByText('Transaction Ledger'))
    await user.click(await screen.findByRole('button', { name: /realized P&L for AAPL/i }))

    await waitFor(() => expect(screen.getByText(/Sell P&L/)).toBeInTheDocument())
    expect(await screen.findByText('2024-01-01')).toBeInTheDocument()
  })

  it('closes the breakdown when the portfolio changes', async () => {
    // The modal renders only while its recorded portfolio still matches the
    // selection, so switching cannot leave figures on screen for a holding the
    // user no longer looks at. Guard introduced in this task; pinned here.
    const user = userEvent.setup()
    server.use(http.get('/api/portfolios/1/transactions', () => HttpResponse.json([
      { id: 10, asset_id: 1, type: 'SELL', quantity: '80.0', price: '130.0', fee: '0.0', date: '2024-12-01T00:00:00Z' },
    ])))
    render(<App />)

    await user.click(await screen.findByText('Transaction Ledger'))
    await user.click(await screen.findByRole('button', { name: /realized P&L for AAPL/i }))
    await screen.findByText(/Sell P&L/)

    await user.selectOptions(screen.getByRole('combobox'), '2')
    await waitFor(() => expect(screen.queryByText(/Sell P&L/)).not.toBeInTheDocument())
  })
})
