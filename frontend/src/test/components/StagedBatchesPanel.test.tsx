import { render, screen, waitFor, fireEvent } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import StagedBatchesPanel from '../../components/StagedBatchesPanel'

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  })
}

const HANDLE = 'a'.repeat(64)

/** A batch waiting for a decision. `expires_at` is a day in the future so the local clock
 * would call it live; the `expired` flag is what the panel must read instead. */
const STAGED = {
  handle: HANDLE,
  status: 'staged',
  created_at: '2026-11-18T10:00:00Z',
  expires_at: new Date(Date.now() + 86_400_000).toISOString(),
  approved_at: null,
  applied_at: null,
  asset_count: 1,
  transaction_count: 3,
  expired: false,
}

/** The impossible row: `expires_at` in the future and `expired` true. No database can
 * produce it, which is the point — the only way this renders as expired is if the panel
 * reads the server's verdict rather than comparing a timestamp to `Date.now()`. */
const SERVER_SAYS_EXPIRED = { ...STAGED, expired: true }

/** The stored preview, including a field this component has never heard of. */
const PREVIEW = {
  target: { New: { name: 'Depot', base_currency: 'EUR' } },
  assets: [
    {
      symbol: 'IWDA',
      transactions: [{ type: 'BUY', date: '2024-03-01', quantity: '1.5', price: '82.34' }],
    },
  ],
  a_field_the_panel_does_not_know: 'still shown',
  status: 'staged',
  expires_at: STAGED.expires_at,
  expired: false,
}

let fetchMock: ReturnType<typeof vi.fn>
let approveStatus = 200
let approveBody: Record<string, unknown> = { status: 'approved' }

beforeEach(() => {
  approveStatus = 200
  approveBody = { status: 'approved' }
  fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input)
    const method = String(init?.method ?? 'GET')
    if (url.endsWith('/staged-batches') && method === 'GET') {
      return jsonResponse([STAGED])
    }
    if (url.includes('/approve') && method === 'POST') {
      return jsonResponse(approveBody, approveStatus)
    }
    if (url.includes('/staged-batches/') && method === 'GET') {
      return jsonResponse(PREVIEW)
    }
    throw new Error(`unexpected request: ${method} ${url}`)
  })
  vi.stubGlobal('fetch', fetchMock)
})

afterEach(() => {
  vi.unstubAllGlobals()
})

describe('StagedBatchesPanel', () => {
  it('shows the server verdict on expiry, not the browser clock', async () => {
    // The row is impossible locally: a future `expires_at` with `expired: true`. If the
    // panel ever derives liveness from `Date.now()` — impure, and a second authority
    // disagreeing with the one that enforces expiry — this is the test that fails.
    fetchMock.mockImplementation(async (input: RequestInfo | URL) => {
      const url = String(input)
      if (url.endsWith('/staged-batches')) return jsonResponse([SERVER_SAYS_EXPIRED])
      // The detail carries the same verdict, exactly as the route computes it: both the
      // list and the detail answer `expired`, and a test that let them disagree would be
      // testing a state the server cannot produce.
      return jsonResponse({ ...PREVIEW, expired: true })
    })

    render(<StagedBatchesPanel />)
    const row = await screen.findByTestId(`staged-row-${HANDLE}`)
    expect(row.textContent).toContain('expired')

    // And the offer to approve is withdrawn, because offering it invites a refusal.
    fireEvent.click(row.querySelector('button')!)
    await waitFor(() => expect(screen.getByTestId('staged-detail')).toBeTruthy())
    expect(screen.queryByTestId('staged-approve')).toBeNull()
  })

  it('renders the stored preview verbatim, including what it does not understand', async () => {
    render(<StagedBatchesPanel />)
    const row = await screen.findByTestId(`staged-row-${HANDLE}`)
    fireEvent.click(row.querySelector('button')!)

    const pre = await screen.findByTestId('staged-preview')
    // A field the component has never seen still appears, which is only true if the panel
    // prints the server's document rather than re-projecting it into a shape of its own.
    // Re-projecting is how a human ends up approving a rendering instead of a proposal.
    expect(pre.textContent).toContain('a_field_the_panel_does_not_know')
    expect(pre.textContent).toContain('still shown')
    expect(pre.textContent).toContain('82.34')
  })

  it('explains a refused approval with the state the server found', async () => {
    approveStatus = 409
    approveBody = { error: 'not_approvable', current_status: 'approved' }
    vi.stubGlobal('confirm', () => true)

    render(<StagedBatchesPanel />)
    const row = await screen.findByTestId(`staged-row-${HANDLE}`)
    fireEvent.click(row.querySelector('button')!)
    await waitFor(() => expect(screen.getByTestId('staged-detail')).toBeTruthy())

    fireEvent.click(screen.getByTestId('staged-approve'))
    const notice = await screen.findByTestId('staged-notice')
    // "already approved" and "gone" are different problems; the panel says which.
    expect(notice.textContent).toContain('approved')
    expect(notice.textContent).toContain('Someone else acted on it first')
  })

  it('says so when nothing is waiting', async () => {
    fetchMock.mockImplementation(async () => jsonResponse([]))
    render(<StagedBatchesPanel />)
    expect(await screen.findByTestId('staged-empty')).toBeTruthy()
  })
})
