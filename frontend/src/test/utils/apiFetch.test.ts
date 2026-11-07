import { afterEach, describe, expect, it, vi } from 'vitest'
import { apiFetch } from '../../utils/apiFetch'

/**
 * `apiFetch` is the only path from this bundle to the backend, so two properties
 * matter and both are easy to break by accident: a `401` must send the user to
 * sign-in, and the caller's `RequestInit` must reach `fetch` unchanged.
 */
describe('apiFetch', () => {
  afterEach(() => {
    vi.unstubAllGlobals()
    vi.restoreAllMocks()
  })

  it('redirects to /auth/login on 401 and still returns the response', async () => {
    const assign = vi.fn()
    vi.spyOn(window, 'location', 'get').mockReturnValue({ ...window.location, assign } as Location)
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(new Response('{"error":"unauthenticated"}', { status: 401 })),
    )

    const response = await apiFetch('/api/portfolios/')

    expect(assign).toHaveBeenCalledWith('/auth/login')
    // Returned rather than swallowed: callers that check `res.ok` must still see
    // the failure instead of a promise that never settles.
    expect(response.status).toBe(401)
  })

  it('does not redirect on 200', async () => {
    const assign = vi.fn()
    vi.spyOn(window, 'location', 'get').mockReturnValue({ ...window.location, assign } as Location)
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response('[]', { status: 200 })))

    await apiFetch('/api/portfolios/')
    expect(assign).not.toHaveBeenCalled()
  })

  it('forwards init verbatim, including the AbortController signal', async () => {
    // `SellPnlModal` passes a `signal` so a closed modal cancels its request. A
    // wrapper that rebuilt the init object would silently drop it and leak the
    // in-flight request; this pins the pass-through rather than the shape.
    const spy = vi.fn().mockResolvedValue(new Response('{}', { status: 200 }))
    vi.stubGlobal('fetch', spy)
    const controller = new AbortController()

    await apiFetch('/api/transactions/1/pnl?strategy=FIFO', {
      signal: controller.signal,
      method: 'POST',
      headers: { 'X-Test': '1' },
    })

    expect(spy).toHaveBeenCalledWith('/api/transactions/1/pnl?strategy=FIFO', {
      signal: controller.signal,
      method: 'POST',
      headers: { 'X-Test': '1' },
    })
  })
})

// The same behaviour driven through MSW rather than a stubbed `fetch`, so the
// handler body is the backend's real `{"error":"unauthenticated"}` rather than
// something this file invented.
import { server } from '../mocks/server'
import { unauthorizedAll } from '../mocks/handlers'

describe('apiFetch against MSW', () => {
  afterEach(() => {
    vi.restoreAllMocks()
  })

  it('redirects when the backend answers 401', async () => {
    server.use(unauthorizedAll)
    const assign = vi.fn()
    vi.spyOn(window, 'location', 'get').mockReturnValue({ ...window.location, assign } as Location)

    const response = await apiFetch('/api/portfolios/')

    expect(response.status).toBe(401)
    expect(await response.clone().json()).toEqual({ error: 'unauthenticated' })
    expect(assign).toHaveBeenCalledWith('/auth/login')
  })
})
