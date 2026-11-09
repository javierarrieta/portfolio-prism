import { describe, it, expect, vi } from 'vitest'
import { act, render, screen, fireEvent, waitFor } from '@testing-library/react'
import SellPnlModal, { REQUEST_TIMEOUT_MS } from '../../components/SellPnlModal'
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

  it('contains the table overflow in its own scroller, not in the dialog', async () => {
    // The dialog sets overflowY for long lot lists, and CSS promotes a `visible`
    // overflow-x to `auto` whenever the other axis is not visible. So a table
    // wider than the dialog hands the whole dialog a horizontal scrollbar -- the
    // bug this pins. The table must sit inside its own scroll container and use the
    // compact cell padding that makes it fit a wide dialog outright.
    renderModal()
    await waitFor(() => expect(screen.getByText(/AAPL/)).toBeInTheDocument())
    const table = screen.getByRole('table')
    expect(table).toHaveClass('data-table--compact')
    expect(table.parentElement).toHaveClass('data-table-container')
    expect(screen.getByTestId('sell-pnl-dialog').contains(table)).toBe(true)
  })

  it('footnotes the FX conversion for a foreign-currency asset', async () => {
    server.use(http.get('/api/transactions/42/pnl', () => HttpResponse.json({
      ...base, currency: 'EUR', asset_currency: 'USD', fx_applied: true,
    })))
    renderModal()
    await waitFor(() => expect(screen.getByTestId('fx-note')).toBeInTheDocument())
    expect(
      screen.getByTestId('fx-note').textContent
    ).toMatch(/rate recorded on or before the sell date/)
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

  it('is a dialog, takes focus, and closes on Escape', async () => {
    const onClose = vi.fn()
    renderModal({ onClose })
    const dialog = await screen.findByRole('dialog')
    expect(dialog).toHaveFocus()
    fireEvent.keyDown(window, { key: 'Escape' })
    expect(onClose).toHaveBeenCalled()
  })

  it('treats a vanished sell as a dead end, not a retry', async () => {
    // 999 is a 404 in the shared handlers. Retrying a deleted sell fails again
    // forever, so the only honest affordance is the message.
    renderModal({ transactionId: 999 })
    await waitFor(() => expect(screen.getByTestId('modal-gone')).toBeInTheDocument())
    expect(screen.queryByRole('button', { name: /retry/i })).not.toBeInTheDocument()
  })

  it('keeps Tab inside the dialog', async () => {
    // aria-modal="true" claims the background is inert, but the browser still Tabs a
    // sighted keyboard user out of the dialog. Shift+Tab from the dialog itself has
    // to wrap to its last control rather than escape behind the overlay.
    renderModal()
    const dialog = await screen.findByRole('dialog')

    fireEvent.keyDown(dialog, { key: 'Tab', shiftKey: true })
    expect(screen.getByRole('button', { name: /close/i })).toHaveFocus()

    fireEvent.keyDown(screen.getByRole('button', { name: /close/i }), { key: 'Tab' })
    expect(dialog).toContainElement(document.activeElement as HTMLElement | null)
  })

  it('gives up on a request that never settles', async () => {
    // A hung request used to leave the spinner up forever with the retry button
    // unreachable behind it. The fake honours the abort signal the way fetch does.
    const real = globalThis.fetch
    vi.stubGlobal('fetch', (_url: unknown, opts: { signal: AbortSignal }) =>
      new Promise((_res, rej) => opts.signal.addEventListener('abort', () => rej(new Error('aborted')))))
    vi.useFakeTimers()
    try {
      renderModal()
      await act(async () => { await vi.advanceTimersByTimeAsync(REQUEST_TIMEOUT_MS) })
      expect(screen.getByTestId('modal-error')).toBeInTheDocument()
    } finally {
      vi.useRealTimers()
      vi.unstubAllGlobals()
      globalThis.fetch = real
    }
  })

  // jsdom implements neither URL.createObjectURL nor Blob.text(), so a download can only
  // be observed at the two seams left open: the object-URL call that hands over the Blob,
  // and the anchor click that triggers it. Both are recorded, nothing is faked. The click
  // record also notes whether the anchor was in the document at that moment, which is the
  // half of the download that Safari and old Firefox were strict about.
  const captureDownload = () => {
    const blobs: Blob[] = []
    const anchors: { el: HTMLAnchorElement; attached: boolean }[] = []
    const url = URL as unknown as Record<string, unknown>
    const saved = { create: url.createObjectURL, revoke: url.revokeObjectURL }
    const revoke = vi.fn()
    url.createObjectURL = (b: Blob) => { blobs.push(b); return 'blob:mock-url' }
    url.revokeObjectURL = revoke
    vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(
      function (this: HTMLAnchorElement) { anchors.push({ el: this, attached: this.isConnected }) })
    return {
      blobs, anchors, revoke,
      restore: () => {
        url.createObjectURL = saved.create
        url.revokeObjectURL = saved.revoke
        vi.restoreAllMocks()
      },
    }
  }

  const readBlob = (blob: Blob) => new Promise<string>((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = () => resolve(String(reader.result))
    reader.onerror = () => reject(reader.error)
    reader.readAsText(blob)
  })

  // The BOM is invisible at the seam above: readAsText performs a UTF-8 decode, and the
  // Encoding spec has that decode consume a leading BOM. That is what a correct consumer
  // does -- and precisely what Excel does not do without the BOM, which is why it has to be
  // in the bytes. So the BOM is pinned on the bytes, and the header on the decoded text.
  const readHeadBytes = (blob: Blob, n: number) => new Promise<number[]>((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = () => resolve([...new Uint8Array(reader.result as ArrayBuffer)])
    reader.onerror = () => reject(reader.error)
    reader.readAsArrayBuffer(blob.slice(0, n))
  })

  it('disables Download while the request is in flight', async () => {
    renderModal()
    expect(screen.getByRole('button', { name: /download csv/i })).toBeDisabled()
    // Let the request land so the modal is not still settling when the test ends.
    await waitFor(() => expect(screen.getByText(/AAPL/)).toBeInTheDocument())
  })

  it('disables Download when the request fails', async () => {
    server.use(http.get('/api/transactions/42/pnl', () => new HttpResponse(null, { status: 500 })))
    renderModal()
    await waitFor(() => expect(screen.getByTestId('modal-error')).toBeInTheDocument())
    expect(screen.getByRole('button', { name: /download csv/i })).toBeDisabled()
  })

  it('disables Download for a vanished sell', async () => {
    renderModal({ transactionId: 999 })
    await waitFor(() => expect(screen.getByTestId('modal-gone')).toBeInTheDocument())
    expect(screen.getByRole('button', { name: /download csv/i })).toBeDisabled()
  })

  it('downloads the breakdown as a file named for the sell', async () => {
    const dl = captureDownload()
    try {
      renderModal()
      await waitFor(() => expect(screen.getByText(/AAPL/)).toBeInTheDocument())
      const button = screen.getByRole('button', { name: /download csv/i })
      expect(button).toBeEnabled()
      fireEvent.click(button)

      expect(dl.anchors).toHaveLength(1)
      // Attached when clicked, gone again by the time the click returns.
      expect(dl.anchors[0].attached).toBe(true)
      expect(dl.anchors[0].el.isConnected).toBe(false)
      expect(dl.anchors[0].el.download).toBe('AAPL_2024-12-01_sell-pnl.csv')
      expect(dl.anchors[0].el.href).toBe('blob:mock-url')
      expect(dl.blobs[0].type).toBe('text/csv;charset=utf-8')

      // The real Blob, read back: this is the payload the user gets, not what the
      // serializer unit test already proved.
      const text = await readBlob(dl.blobs[0])
      // UTF-8 BOM (EF BB BF) as the first three bytes -- see utils/download.ts for why it is
      // there. "Exactly once" is proved by the line *after* this one, not by the byte check:
      // a UTF-8 decode consumes only the first BOM, so a doubled one would leave `\uFEFF`
      // at the front of `text`. The guard itself is pinned in utils/download.test.ts.
      expect(await readHeadBytes(dl.blobs[0], 3)).toEqual([0xef, 0xbb, 0xbf])
      expect(text.startsWith('row_type,transaction_id')).toBe(true)
      expect(text).toContain('2024-01-01T00:00:00Z')
      // Revocation is deferred one task on purpose (see utils/download.ts), so it has to be
      // waited for rather than asserted on the line after the click.
      await waitFor(() => expect(dl.revoke).toHaveBeenCalledWith('blob:mock-url'))
    } finally {
      dl.restore()
    }
  })

  it('reaches Download by Tab and keeps Shift+Tab inside the dialog', async () => {
    // Download is placed before Close in the header, so it becomes the trap's first control
    // and Close stays the last -- which is why the existing wrap test needed no edit.
    renderModal()
    // Wait for the payload, not just for the dialog: the dialog renders while still
    // loading, and a disabled button silently refuses focus, which made this flaky.
    await waitFor(() => expect(screen.getByText(/AAPL/)).toBeInTheDocument())
    const dialog = screen.getByRole('dialog')
    const download = screen.getByRole('button', { name: /download csv/i })
    const close = screen.getByRole('button', { name: /close/i })
    expect(download).toBeEnabled()

    expect(dialog).toContainElement(download)
    download.focus()
    expect(download).toHaveFocus()

    fireEvent.keyDown(download, { key: 'Tab', shiftKey: true })
    expect(close).toHaveFocus()

    fireEvent.keyDown(close, { key: 'Tab' })
    expect(dialog).toContainElement(document.activeElement as HTMLElement | null)
  })
})
