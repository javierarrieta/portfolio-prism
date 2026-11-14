import { describe, it, expect } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { http, HttpResponse } from 'msw'
import App from '../../App'
import { server } from '../mocks/server'

describe('the backup tab', () => {
  it('opens the export card from the sidebar, over the portfolios App already has', async () => {
    server.use(http.get('/api/backup/capabilities', () => HttpResponse.json({
      schema_versions: [1], allow_price_restore: false, max_bytes: 8388608, max_rows: 50000,
    })))
    render(<App />)
    await userEvent.click(await screen.findByText('Backup'))

    await waitFor(() => expect(screen.getByTestId('export-card')).toBeInTheDocument())
    // The panel is handed the portfolios App already had, so the checkboxes are the
    // user's own and not a second list the tab went and fetched for itself.
    expect(screen.getByRole('checkbox', { name: /Test Portfolio/i })).toBeInTheDocument()
    expect(screen.getByRole('checkbox', { name: /Retirement Fund/i })).toBeInTheDocument()
    // The capabilities read is the tab's own, and its limits reach the screen.
    expect(await screen.findByTestId('caps-note')).toHaveTextContent('50,000 rows')
  })

  it('opens the export card for a user who owns no portfolios', async () => {
    server.use(
      http.get('/api/portfolios', () => HttpResponse.json([])),
      http.get('/api/backup/capabilities', () => HttpResponse.json({
        schema_versions: [1], allow_price_restore: false, max_bytes: 8388608, max_rows: 50000,
      })),
    )
    render(<App />)
    await userEvent.click(await screen.findByText('Backup'))

    // Export is account-level, so the per-portfolio guard must not stand in front of
    // it. This is also the only way into the restore card that Task 12 adds, and the
    // user who needs restore is exactly the one with nothing to select: a fresh
    // account, or a replace that went wrong.
    expect(await screen.findByTestId('export-card')).toBeInTheDocument()
    expect(screen.getByTestId('no-portfolios')).toBeInTheDocument()
    expect(screen.queryByText('No Portfolio Active')).toBeNull()
  })
})
