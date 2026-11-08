import { describe, expect, it, beforeEach } from 'vitest'
import { render, screen } from '@testing-library/react'
import App from '../../App'

/**
 * The sign-in failure banner.
 *
 * The backend emits exactly one value, `?auth_error=auth_failed`, and reflects no
 * IdP detail. The page must show a fixed message for it and remove the parameter,
 * so a refresh or a shared link does not resurrect a failure already resolved.
 */
describe('sign-in failure banner', () => {
  beforeEach(() => {
    window.history.replaceState({}, '', '/?auth_error=auth_failed')
  })

  it('renders the fixed message and strips the parameter', async () => {
    render(<App />)

    expect(await screen.findByText(/Sign-in failed/i)).toBeInTheDocument()
    // Stripped with replaceState, so the URL no longer carries the parameter while
    // the page is still showing the message.
    expect(window.location.search).not.toContain('auth_error')
    expect(window.location.pathname).toBe('/')
  })

  it('does not render when the parameter is absent', async () => {
    window.history.replaceState({}, '', '/')
    render(<App />)

    // Any other error surface may appear; the auth banner specifically must not.
    expect(screen.queryByText(/Sign-in failed/i)).not.toBeInTheDocument()
  })

  it('ignores a value other than auth_failed', async () => {
    // Only the one value the backend emits is ever honoured; anything else in the
    // query string is not a message to display.
    window.history.replaceState({}, '', '/?auth_error=something_else')
    render(<App />)

    expect(screen.queryByText(/Sign-in failed/i)).not.toBeInTheDocument()
    expect(window.location.search).toContain('auth_error=something_else')
  })
})
