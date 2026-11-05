import { describe, it, expect, vi } from 'vitest'
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import SellPnlModal from '../../components/SellPnlModal'
import { server } from '../mocks/server'
import { http, HttpResponse } from 'msw'

const base = {
  transaction_id: 42, asset_id: 1, symbol: 'AAPL', asset_type: 'STOCK',
  strategy: 'FIFO', threshold_days: 30,
  currency: 'USD', asset_currency: 'USD', fx_applied: false, split_applied: false,
  sell_date: '2024-12-01T00:00:00Z',
  sold_qty: '80.0', price: '130.0', fee: '0.0',
  proceeds: '10400.0', cost_basis: '8400.0', realized_pnl: '2000.0', roi: '0.238',
  matched_qty: '80.0', unmatched_qty: '0.0', matches: [],
}

const renderModal = (props = {}) => render(
  <SellPnlModal
    transactionId={42}
    strategy="FIFO"
    thresholdDays={30}
    onClose={vi.fn()}
    {...props}
  />,
)

describe('SellPnlModal', () => {
  it('shows the sell summary and every consumed lot', async () => {
    renderModal()
    // The symbol lives inside the heading text "Sell P&L — AAPL", so match it
    // as a substring; getByText with a bare string compares whole text content.
    await waitFor(() => expect(screen.getByText(/AAPL/)).toBeInTheDocument())
    expect(screen.getByText(/2,000/)).toBeInTheDocument()
    expect(screen.getByText('2024-01-01')).toBeInTheDocument()
    expect(screen.getByText('2024-06-01')).toBeInTheDocument()
    expect(screen.getByTestId('strategy-note')).toHaveTextContent('realized · FIFO')
  })

  it('footnotes the FX conversion for a foreign-currency asset', async () => {
    server.use(http.get('/api/transactions/42/pnl', () => HttpResponse.json({
      ...base, currency: 'EUR', asset_currency: 'USD', fx_applied: true,
    })))
    renderModal()
    await waitFor(() => expect(screen.getByTestId('fx-note')).toBeInTheDocument())
    expect(screen.getByTestId('fx-note')).toHaveTextContent('converted to EUR')
  })

  it('names the unmatched quantity instead of hiding it', async () => {
    server.use(http.get('/api/transactions/42/pnl', () => HttpResponse.json({
      ...base, sold_qty: '100.0', matched_qty: '50.0', unmatched_qty: '50.0', realized_pnl: '1000.0',
    })))
    renderModal()
    await waitFor(() => expect(screen.getByTestId('unmatched-banner')).toBeInTheDocument())
    expect(screen.getByTestId('unmatched-banner')).toHaveTextContent('50 of 100')
    // The wording is the finding, not the counts: the engine drops unmatched units from BOTH
    // sides, so the figure is incomplete. "overstated"/"understated" would be an unsupported claim.
    expect(screen.getByTestId('unmatched-banner')).toHaveTextContent(
      'Neither their cost basis nor their proceeds are included',
    )
    expect(screen.getByTestId('unmatched-banner')).not.toHaveTextContent('overstated')
  })

  it('explains a sell with nothing to match', async () => {
    server.use(http.get('/api/transactions/42/pnl', () => HttpResponse.json({
      ...base, sold_qty: '50.0', matched_qty: '0.0', unmatched_qty: '50.0',
      cost_basis: '0.0', proceeds: '0.0', realized_pnl: '0.0', roi: null,
    })))
    renderModal()
    await waitFor(() => expect(screen.getByTestId('no-lots-note')).toBeInTheDocument())
    expect(screen.getByTestId('no-lots-note')).toHaveTextContent('No buy lots were available to match')
  })

  it('renders roi as an em dash when cost basis is zero', async () => {
    server.use(http.get('/api/transactions/42/pnl', () => HttpResponse.json({
      ...base, cost_basis: '0.0', realized_pnl: '0.0', roi: null,
    })))
    renderModal()
    await waitFor(() => expect(screen.getByTestId('roi')).toBeInTheDocument())
    expect(screen.getByTestId('roi')).toHaveTextContent('—')
    expect(screen.getByTestId('roi')).not.toHaveTextContent('NaN')
    expect(screen.getByTestId('roi')).not.toHaveTextContent('Infinity')
  })

  it('footnotes split adjustment only when the response says a split applies', async () => {
    // Driven by the payload, not by a prop the caller computes: the server knows
    // whether a split touched this asset because it read the same rows the
    // numbers came from. `base` carries split_applied: false.
    renderModal()
    await waitFor(() => expect(screen.queryByTestId('split-note')).not.toBeInTheDocument())

    server.use(http.get('/api/transactions/42/pnl', () => HttpResponse.json({
      ...base, split_applied: true,
    })))
    renderModal()
    await waitFor(() => expect(screen.getByTestId('split-note')).toBeInTheDocument())
  })

  it('shows an error and retries', async () => {
    server.use(http.get('/api/transactions/42/pnl', () => new HttpResponse(null, { status: 500 })))
    renderModal()
    await waitFor(() => expect(screen.getByTestId('modal-error')).toBeInTheDocument())
    server.use(http.get('/api/transactions/42/pnl', () => HttpResponse.json(base)))
    fireEvent.click(screen.getByRole('button', { name: /retry/i }))
    await waitFor(() => expect(screen.getByText(/AAPL/)).toBeInTheDocument())
  })

  it('calls onClose when dismissed', async () => {
    const onClose = vi.fn()
    renderModal({ onClose })
    await waitFor(() => expect(screen.getByText(/AAPL/)).toBeInTheDocument())
    fireEvent.click(screen.getByRole('button', { name: /close/i }))
    expect(onClose).toHaveBeenCalled()
  })
})