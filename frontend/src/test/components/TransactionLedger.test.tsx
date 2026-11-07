import { describe, it, expect, vi } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import TransactionLedger from '../../components/TransactionLedger'
import type { Asset, Transaction } from '../../types'

const assets: Asset[] = [
  { id: 1, portfolio_id: 1, symbol: 'AAPL', name: 'Apple Inc.', asset_type: 'STOCK', currency: 'USD', isin: null, transactions: [] },
]

const txs: Transaction[] = [
  { id: 10, asset_id: 1, type: 'SELL', quantity: 80, price: 130, fee: 0, date: '2024-12-01T00:00:00Z' },
  { id: 11, asset_id: 1, type: 'BUY', quantity: 60, price: 100, fee: 0, date: '2024-01-01T00:00:00Z' },
  { id: 12, asset_id: 1, type: 'SPLIT', quantity: 2, price: 1, fee: 0, date: '2024-06-01T00:00:00Z' },
]

const noop = async () => {}

const renderLedger = (onSelectSell = vi.fn()) => {
  render(
    <TransactionLedger
      transactions={txs}
      assets={assets}
      onDeleteTransaction={noop}
      onSelectSell={onSelectSell}
    />,
  )
  return onSelectSell
}

describe('TransactionLedger', () => {
  it('renders every transaction', () => {
    renderLedger()
    expect(screen.getAllByRole('row')).toHaveLength(4)
  })

  it('offers a realized P&L control on a SELL row and fires with its id', () => {
    const onSelectSell = renderLedger()
    const btn = screen.getByRole('button', { name: /realized P&L for AAPL/i })
    fireEvent.click(btn)
    expect(onSelectSell).toHaveBeenCalledWith(10)
  })

  it('offers no P&L control on BUY or SPLIT rows', () => {
    renderLedger()
    expect(screen.getAllByRole('button', { name: /realized P&L/i })).toHaveLength(1)
  })

  it('clicking a SELL row body also opens the breakdown', () => {
    const onSelectSell = renderLedger()
    fireEvent.click(screen.getByText('SELL'))
    expect(onSelectSell).toHaveBeenCalledWith(10)
  })

  it('clicking a BUY row body does nothing', () => {
    const onSelectSell = renderLedger()
    fireEvent.click(screen.getByText('BUY'))
    expect(onSelectSell).not.toHaveBeenCalled()
  })

  it('still renders the delete control for every row', () => {
    renderLedger()
    expect(screen.getAllByTitle('Delete transaction')).toHaveLength(3)
  })
})