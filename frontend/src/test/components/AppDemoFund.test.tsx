import { describe, it, expect } from 'vitest'
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import { http, HttpResponse } from 'msw'
import App from '../../App'
import { server } from '../mocks/server'

/**
 * "Load Demo Fund" was broken and no test knew.
 *
 * `PortfolioCreate.currency` is a required `String`, so the real server answers 422 when it
 * is absent — and the demo loader never sent it. The test double is what hid it:
 * `handlers.ts` defaults `currency: body.currency || 'USD'`, so the mock accepted the
 * incomplete body and answered 201. A double that fills in what the real server demands will
 * pass any client that forgets to send it.
 *
 * The second test is about the error, not the bug. Rocket returns an HTML page for a
 * rejected request, and the loader called `.json()` on it unconditionally, so every failure
 * — whatever it was — arrived as "Unexpected token '<'". The status was thrown away before
 * anyone could read it.
 */
// The button is rendered only while `portfolios.length === 0`, and the default double
// answers with a portfolio — so it unmounts a tick after first paint and a click aimed at
// it lands on a detached node. Empty the list first and the button stays put.
const emptyListAndClickDemo = async () => {
  server.use(http.get('/api/portfolios', () => HttpResponse.json([])))
  fireEvent.click(await screen.findByText('Load Demo Fund'))
}

describe('App demo fund loader', () => {
  it('sends every field the real server requires', async () => {
    const sent: Record<string, unknown>[] = []
    server.use(
      http.post('/api/portfolios', async ({ request }) => {
        const body = (await request.json()) as Record<string, unknown>
        sent.push(body)
        return HttpResponse.json(
          { id: 7, name: body.name, description: body.description, currency: body.currency, assets: [] },
          { status: 201 },
        )
      }),
    )

    render(<App />)
    await emptyListAndClickDemo()
    await waitFor(() => expect(sent.length).toBeGreaterThan(0))

    // Not `expect(body.currency).toBeDefined()` — the field existing as null or '' would
    // still be a 422, because the server wants a non-empty String.
    expect(String(sent[0].currency)).toMatch(/^[A-Z]{3}$/)
  })

  it('names the status when the server refuses, instead of a JSON parse error', async () => {
    server.use(
      http.post(
        '/api/portfolios',
        () =>
          new HttpResponse('<!DOCTYPE html><html><body><h1>500 Internal Server Error</h1></body></html>', {
            status: 500,
            headers: { 'Content-Type': 'text/html' },
          }),
      ),
    )

    render(<App />)
    await emptyListAndClickDemo()

    const shown = await screen.findByText(/could not create the demo portfolio/)
    expect(shown.textContent).toContain('HTTP 500')
    expect(shown.textContent).not.toContain('Unexpected token')
  })
})
