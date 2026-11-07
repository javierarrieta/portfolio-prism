import { describe, expect, it } from 'vitest'
import { render, screen } from '@testing-library/react'
import App from '../../App'

/**
 * The sign-out affordance.
 *
 * The backend route has existed since the auth work landed, but nothing linked to
 * it, so no user could reach it. What is pinned here is the shape that makes it
 * work: a real navigation to `/auth/logout`, not a `fetch`. Logout leaves the
 * origin for the identity provider and comes back, and a `fetch` would follow those
 * redirects invisibly while the browser's session at the provider stayed open.
 */
describe('sign out', () => {
  it('links to the backend logout route as a real navigation', () => {
    render(<App />)

    const link = screen.getByRole('link', { name: /sign out/i })
    expect(link).toHaveAttribute('href', '/auth/logout')
  })

  it('is reachable without selecting a portfolio', () => {
    // The MSW fixtures start with no portfolios on some suites; logout is not a
    // per-portfolio action and must not disappear with them.
    render(<App />)
    expect(screen.getByRole('link', { name: /sign out/i })).toBeInTheDocument()
  })
})
