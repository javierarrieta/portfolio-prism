import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { render, screen, waitFor, within } from '@testing-library/react'
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

  it('closes the breakdown when leaving the ledger tab', async () => {
    // The breakdown is about a ledger row. Left rendered, it floats over Dashboard
    // -- a modal about a transaction on a screen showing no transactions.
    const user = userEvent.setup()
    server.use(http.get('/api/portfolios/1/transactions', () => HttpResponse.json([
      { id: 10, asset_id: 1, type: 'SELL', quantity: '80.0', price: '130.0', fee: '0.0', date: '2024-12-01T00:00:00Z' },
    ])))
    render(<App />)

    await user.click(await screen.findByText('Transaction Ledger'))
    await user.click(await screen.findByRole('button', { name: /realized P&L for AAPL/i }))
    await screen.findByRole('dialog')

    await user.click(await screen.findByText('Dashboard'))
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
  })

  // Locate a row's delete button by the row's own Type cell. Index-based lookups
  // silently target the wrong row if the ledger ever changes its sort order.
  const deleteButtonFor = async (type: 'SELL' | 'BUY') => {
    const cell = await screen.findByText(type)
    return within(cell.closest('tr') as HTMLElement).findByTitle('Delete transaction')
  }

  const twoRows = (onList?: () => void) =>
    http.get('/api/portfolios/1/transactions', () => {
      onList?.()
      return HttpResponse.json([
        { id: 10, asset_id: 1, type: 'SELL', quantity: '80.0', price: '130.0', fee: '0.0', date: '2024-12-01T00:00:00Z' },
        { id: 11, asset_id: 1, type: 'BUY', quantity: '60.0', price: '100.0', fee: '0.0', date: '2024-01-01T00:00:00Z' },
      ])
    })
  it('keeps the breakdown open when an unrelated row is deleted', async () => {
    // Deleting some other transaction must not dismiss a figure someone is reading.
    // Local spy: without a truthy confirm, handleDeleteTransaction returns early and
    // the test would pass by doing nothing.
    vi.spyOn(window, 'confirm').mockReturnValue(true)
    let listed = 0
    const dels: string[] = []
    const user = userEvent.setup()
    server.use(twoRows(() => { listed += 1 }),
      http.delete('/api/transactions/:id', ({ params }) => {
        dels.push(String(params.id))
        return new HttpResponse(null, { status: 204 })
      }))
    render(<App />)

    await user.click(await screen.findByText('Transaction Ledger'))
    await user.click(await screen.findByRole('button', { name: /realized P&L for AAPL/i }))
    await screen.findByRole('dialog')
    const before = listed

    await user.click(await deleteButtonFor('BUY'))
    // Prove the delete round-trip finished before asserting the dialog survived;
    // asserting immediately would pass even if the modal closed a tick later.
    await waitFor(() => expect(dels).toEqual(['11']))
    await waitFor(() => expect(listed).toBeGreaterThan(before))
    expect(screen.getByRole('dialog')).toBeInTheDocument()
  })

  it('closes the breakdown when the sell it shows is deleted', async () => {
    vi.spyOn(window, 'confirm').mockReturnValue(true)
    const user = userEvent.setup()
    server.use(twoRows(),
      http.delete('/api/transactions/:id', ({ params }) => {
        if (params.id === '10') return new HttpResponse(null, { status: 204 })
        return new HttpResponse(null, { status: 501 })
      }))
    render(<App />)

    await user.click(await screen.findByText('Transaction Ledger'))
    await user.click(await screen.findByRole('button', { name: /realized P&L for AAPL/i }))
    await screen.findByRole('dialog')

    await user.click(await deleteButtonFor('SELL'))
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument())
  })
})
