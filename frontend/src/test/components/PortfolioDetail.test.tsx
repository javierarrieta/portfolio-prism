import { describe, it, expect, vi } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import PortfolioDetail from '../../components/PortfolioDetail'
import type { Portfolio, TaxSummary } from '../../types'

const mockPortfolio: Portfolio = {
  id: 1,
  name: 'Test Portfolio',
  description: 'A test portfolio',
  currency: 'USD',
  assets: [
    {
      id: 1,
      portfolio_id: 1,
      symbol: 'AAPL',
      name: 'Apple Inc.',
      asset_type: 'STOCK',
      sector: 'Technology',
      currency: 'USD',
      isin: null,
      transactions: [],
    },
  ],
}

const mockTaxSummary: TaxSummary = {
  assets: [
    {
      symbol: 'AAPL',
      asset_type: 'STOCK',
      current_shares: 100,
      average_cost: 150,
      current_price: 175,
      total_cost: 15000,
      market_value: 17500,
      unrealized_pnl: 2500,
      unrealized_roi: 0.1667,
      realized_pnl: 0,
      tax_lots: [],
    },
  ],
  currency: 'USD',
  total_portfolio_value: 17500,
  total_realized_pnl: 0,
  total_unrealized_pnl: 2500,
}

const emptyTaxSummary: TaxSummary = {
  assets: [],
  currency: 'USD',
  total_portfolio_value: 0,
  total_realized_pnl: 0,
  total_unrealized_pnl: 0,
}

const noop = async () => {}

describe('PortfolioDetail', () => {
  it('shows "Log Transaction" button when assets exist', () => {
    render(
      <PortfolioDetail
        onFetchPortfolioData={noop}
        portfolio={mockPortfolio}
        taxSummary={mockTaxSummary}
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

    expect(screen.getByText('Log Transaction')).toBeVisible()
  })

  it('hides "Log Transaction" button when no assets exist', () => {
    render(
      <PortfolioDetail
        onFetchPortfolioData={noop}
        portfolio={{ ...mockPortfolio, assets: [] }}
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

    expect(screen.queryByText('Log Transaction')).not.toBeInTheDocument()
  })

  it('shows "Add Asset Symbol" button always', () => {
    render(
      <PortfolioDetail
        onFetchPortfolioData={noop}
        portfolio={{ ...mockPortfolio, assets: [] }}
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

    expect(screen.getByText('Add Asset Symbol')).toBeVisible()
  })

  it('opens transaction modal when "Log Transaction" is clicked', async () => {
    const user = userEvent.setup()
    render(
      <PortfolioDetail
        onFetchPortfolioData={noop}
        portfolio={mockPortfolio}
        taxSummary={mockTaxSummary}
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

    await user.click(screen.getByText('Log Transaction'))

    expect(screen.getByText('Log Buy/Sell/Split Transaction')).toBeVisible()
  })

  it('opens asset modal when "Add Asset Symbol" is clicked', async () => {
    const user = userEvent.setup()
    render(
      <PortfolioDetail
        onFetchPortfolioData={noop}
        portfolio={{ ...mockPortfolio, assets: [] }}
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

    await user.click(screen.getByText('Add Asset Symbol'))

    expect(screen.getByText('Register New Asset Symbol')).toBeVisible()
  })

  it('renders tax lot details when asset row is expanded', () => {
    const taxSummaryWithLots: TaxSummary = {
      assets: [
        {
          symbol: 'AAPL',
          asset_type: 'STOCK',
          current_shares: 100,
          average_cost: 150,
          current_price: 175,
          total_cost: 15000,
          market_value: 17500,
          unrealized_pnl: 2500,
          unrealized_roi: 0.1667,
          realized_pnl: 0,
          tax_lots: [
            {
              buy_transaction_id: 1,
              buy_date: '2024-01-15T00:00:00Z',
              buy_price: 150,
              original_qty: 50,
              remaining_qty: 30,
              latent_gain_loss: 750,
              latent_roi: 0.1667,
            },
          ],
        },
      ],
      currency: 'USD',
      total_portfolio_value: 17500,
      total_realized_pnl: 0,
      total_unrealized_pnl: 2500,
    }

    render(
      <PortfolioDetail
        onFetchPortfolioData={noop}
        portfolio={mockPortfolio}
        taxSummary={taxSummaryWithLots}
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

    const aaplRow = screen.getByText('AAPL').closest('tr')
    expect(aaplRow).toBeDefined()
    const expandButton = aaplRow!.querySelector('button')
    expect(expandButton).toBeDefined()
    fireEvent.click(expandButton!)
    expect(screen.getByText('Tax Lots (Unrealized Profit Details)')).toBeVisible()

    // The two quantity columns must read from two different fields. The engine used to
    // send the same already-decremented number for both, so the fixture gives them
    // different values (remaining 30, original 50) and the cells are asserted -- without
    // these two lines, putting `lot.remaining_qty` back in the Original Qty cell is
    // invisible to the whole frontend suite.
    // Anchored on the header because the <h4> heading is a sibling of this inner table,
    // so `closest('table')` from it would land on the outer holdings table.
    const lotTable = screen.getByText('Original Qty').closest('table')!
    const lotCells = Array.from(lotTable.querySelectorAll('tbody tr td')).map((td) => td.textContent)
    expect(lotCells[2]).toBe('30') // Remaining Qty  <- lot.remaining_qty
    expect(lotCells[3]).toBe('50') // Original Qty   <- lot.original_qty
    expect(lotCells[2]).not.toBe(lotCells[3])
  })

  it('shows empty state message when no assets are registered', () => {
    render(
      <PortfolioDetail
        onFetchPortfolioData={noop}
        portfolio={{ ...mockPortfolio, assets: [] }}
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

    expect(
      screen.getByText('No assets registered yet. Click "Add Asset Symbol" to begin.')
    ).toBeVisible()
  })

  it('allows selecting BUY or SELL in transaction modal', async () => {
    const user = userEvent.setup()
    render(
      <PortfolioDetail
        onFetchPortfolioData={noop}
        portfolio={mockPortfolio}
        taxSummary={mockTaxSummary}
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

    await user.click(screen.getByText('Log Transaction'))
    expect(screen.getByText('Log Buy/Sell/Split Transaction')).toBeVisible()

    const buyOption = screen.getByText('BUY')
    const typeSelect = buyOption.closest('select') as HTMLSelectElement
    expect(typeSelect).toBeDefined()
    expect(typeSelect.value).toBe('BUY')

    await user.selectOptions(typeSelect, 'SELL')
    expect(typeSelect.value).toBe('SELL')
  })

  it('allows selecting SPLIT in transaction modal and hides fee', async () => {
    const user = userEvent.setup()
    render(
      <PortfolioDetail
        onFetchPortfolioData={noop}
        portfolio={mockPortfolio}
        taxSummary={mockTaxSummary}
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

    await user.click(screen.getByText('Log Transaction'))
    expect(screen.getByText('Log Buy/Sell/Split Transaction')).toBeVisible()

    const buyOption = screen.getByText('BUY')
    const typeSelect = buyOption.closest('select') as HTMLSelectElement
    await user.selectOptions(typeSelect, 'SPLIT')
    expect(typeSelect.value).toBe('SPLIT')

    expect(screen.getByText(/Shares after : Shares before/i)).toBeVisible()
    expect(screen.queryByText(/Transaction Fee/i)).not.toBeInTheDocument()
  })

  it('calls onAddTransaction when transaction form is submitted', async () => {
    const user = userEvent.setup()
    const onAddTransaction = vi.fn().mockResolvedValue(undefined)

    render(
      <PortfolioDetail
        onFetchPortfolioData={noop}
        portfolio={mockPortfolio}
        taxSummary={mockTaxSummary}
        onAddAsset={noop}
        onDeleteAsset={noop}
        onDeletePortfolio={noop}
        onAddTransaction={onAddTransaction}
        strategy="FIFO"
        setStrategy={() => {}}
        thresholdDays={30}
        setThresholdDays={() => {}}
      />
    )

    // Open the transaction modal
    await user.click(screen.getAllByText('Log Transaction')[0])
    expect(screen.getByText('Log Buy/Sell/Split Transaction')).toBeVisible()

    // Select asset
    const aaplOption = screen.getByText('AAPL (Apple Inc.)')
    const assetSelect = aaplOption.closest('select') as HTMLSelectElement
    await user.selectOptions(assetSelect, '1')

    // Find the form and submit button within it
    const form = document.querySelector('form')
    expect(form).toBeDefined()
    const submitBtn = form!.querySelector('button[type="submit"]')
    expect(submitBtn).toBeDefined()

    // Set values on controlled inputs and dispatch React-compatible events
    const spinbuttons = screen.getAllByRole('spinbutton') as HTMLInputElement[]
    const quantityInput = spinbuttons[0]
    const priceInput = spinbuttons[1]

    // Simulate React's onChange by setting value and dispatching input event
    const nativeInputValueSetter = Object.getOwnPropertyDescriptor(
      window.HTMLInputElement.prototype, 'value'
    )?.set
    nativeInputValueSetter?.call(quantityInput, '10')
    quantityInput.dispatchEvent(new Event('input', { bubbles: true }))
    quantityInput.dispatchEvent(new Event('change', { bubbles: true }))

    nativeInputValueSetter?.call(priceInput, '180')
    priceInput.dispatchEvent(new Event('input', { bubbles: true }))
    priceInput.dispatchEvent(new Event('change', { bubbles: true }))

    await user.click(submitBtn!)

    expect(onAddTransaction).toHaveBeenCalledWith(1, expect.objectContaining({
      type: 'BUY',
      quantity: '10',
      price: '180',
    }))
  })

  it('calls onAddTransaction with fee "0" when a valid SPLIT is submitted', async () => {
    const user = userEvent.setup()
    const onAddTransaction = vi.fn().mockResolvedValue(undefined)

    render(
      <PortfolioDetail
        onFetchPortfolioData={noop}
        portfolio={mockPortfolio}
        taxSummary={mockTaxSummary}
        onAddAsset={noop}
        onDeleteAsset={noop}
        onDeletePortfolio={noop}
        onAddTransaction={onAddTransaction}
        strategy="FIFO"
        setStrategy={() => {}}
        thresholdDays={30}
        setThresholdDays={() => {}}
      />
    )

    await user.click(screen.getAllByText('Log Transaction')[0])
    expect(screen.getByText('Log Buy/Sell/Split Transaction')).toBeVisible()

    const typeSelect = screen.getByText('BUY').closest('select') as HTMLSelectElement
    await user.selectOptions(typeSelect, 'SPLIT')

    const aaplOption = screen.getByText('AAPL (Apple Inc.)')
    const assetSelect = aaplOption.closest('select') as HTMLSelectElement
    await user.selectOptions(assetSelect, '1')

    const form = document.querySelector('form')
    const submitBtn = form!.querySelector('button[type="submit"]')

    const spinbuttons = screen.getAllByRole('spinbutton') as HTMLInputElement[]
    const quantityInput = spinbuttons[0]
    const priceInput = spinbuttons[1]
    const nativeInputValueSetter = Object.getOwnPropertyDescriptor(
      window.HTMLInputElement.prototype, 'value'
    )?.set
    nativeInputValueSetter?.call(quantityInput, '2')
    quantityInput.dispatchEvent(new Event('input', { bubbles: true }))
    quantityInput.dispatchEvent(new Event('change', { bubbles: true }))
    nativeInputValueSetter?.call(priceInput, '1')
    priceInput.dispatchEvent(new Event('input', { bubbles: true }))
    priceInput.dispatchEvent(new Event('change', { bubbles: true }))

    await user.click(submitBtn!)

    expect(onAddTransaction).toHaveBeenCalledWith(1, expect.objectContaining({
      type: 'SPLIT',
      quantity: '2',
      price: '1',
      fee: '0',
    }))
  })

  it('shows an error and does not submit when a SPLIT has a non-positive ratio', async () => {
    const user = userEvent.setup()
    const onAddTransaction = vi.fn().mockResolvedValue(undefined)

    render(
      <PortfolioDetail
        onFetchPortfolioData={noop}
        portfolio={mockPortfolio}
        taxSummary={mockTaxSummary}
        onAddAsset={noop}
        onDeleteAsset={noop}
        onDeletePortfolio={noop}
        onAddTransaction={onAddTransaction}
        strategy="FIFO"
        setStrategy={() => {}}
        thresholdDays={30}
        setThresholdDays={() => {}}
      />
    )

    await user.click(screen.getAllByText('Log Transaction')[0])
    expect(screen.getByText('Log Buy/Sell/Split Transaction')).toBeVisible()

    const typeSelect = screen.getByText('BUY').closest('select') as HTMLSelectElement
    await user.selectOptions(typeSelect, 'SPLIT')

    const aaplOption = screen.getByText('AAPL (Apple Inc.)')
    const assetSelect = aaplOption.closest('select') as HTMLSelectElement
    await user.selectOptions(assetSelect, '1')

    const form = document.querySelector('form')
    const submitBtn = form!.querySelector('button[type="submit"]')

    const spinbuttons = screen.getAllByRole('spinbutton') as HTMLInputElement[]
    const quantityInput = spinbuttons[0]
    const priceInput = spinbuttons[1]
    const nativeInputValueSetter = Object.getOwnPropertyDescriptor(
      window.HTMLInputElement.prototype, 'value'
    )?.set
    nativeInputValueSetter?.call(quantityInput, '0')
    quantityInput.dispatchEvent(new Event('input', { bubbles: true }))
    quantityInput.dispatchEvent(new Event('change', { bubbles: true }))
    nativeInputValueSetter?.call(priceInput, '1')
    priceInput.dispatchEvent(new Event('input', { bubbles: true }))
    priceInput.dispatchEvent(new Event('change', { bubbles: true }))

    await user.click(submitBtn!)

    expect(onAddTransaction).not.toHaveBeenCalled()
    expect(screen.getByText(/Split ratio must be positive/i)).toBeInTheDocument()
  })

  it('does not show Delete Portfolio button when portfolio has assets', () => {
    render(
      <PortfolioDetail
        onFetchPortfolioData={noop}
        portfolio={mockPortfolio}
        taxSummary={mockTaxSummary}
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

    expect(screen.queryByText('Delete Portfolio')).not.toBeInTheDocument()
  })

  it('shows Delete Portfolio button when portfolio has no assets', () => {
    render(
      <PortfolioDetail
        onFetchPortfolioData={noop}
        portfolio={{ ...mockPortfolio, assets: [] }}
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

    expect(screen.getByText('Delete Portfolio')).toBeInTheDocument()
  })

  it('includes ISIN field in the Add Asset modal', async () => {
    const onAddAsset = vi.fn().mockResolvedValue(undefined)
    render(
      <PortfolioDetail
        portfolio={mockPortfolio}
        taxSummary={mockTaxSummary}
        onAddAsset={onAddAsset}
        onDeleteAsset={noop}
        onDeletePortfolio={noop}
        onFetchPortfolioData={noop}
        onAddTransaction={noop}
        strategy="FIFO"
        setStrategy={() => {}}
        thresholdDays={30}
        setThresholdDays={() => {}}
      />
    )

    const addButton = screen.getByText('Add Asset Symbol')
    fireEvent.click(addButton)

    expect(screen.getByPlaceholderText('e.g. US0378331005')).toBeInTheDocument()
  })
})
