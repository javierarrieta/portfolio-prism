import { describe, it, expect, afterEach, vi } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import App from '../../App'

/**
 * The collapsible nav rail.
 *
 * What is pinned here is the part a stylesheet cannot be trusted with: that a collapsed
 * rail is still a navigable one. Every nav item keeps an accessible name (the label is
 * clipped by CSS, never unmounted, so the name survives the collapse), the toggle says
 * which way it goes, the choice is remembered, and the things the rail genuinely drops --
 * the portfolio picker and the creation actions -- are dropped from the tab order too.
 *
 * The visual half (64px, the clip, the raised content cap) lives in NavRailCss.test.ts,
 * because vitest does not load index.css into jsdom and no DOM assertion here can see it.
 */

const KEY = 'portfolio-prism:nav-collapsed'

const NAV_LABELS = [
  'Dashboard',
  'Holdings & Tax lots',
  'Risk & Correlation',
  'Transaction Ledger',
  'Printable Statement',
  'Tax Report',
  'Backup',
  'Agent staging',
  // Account-level like Backup, and pinned here because this list is the only thing
  // that notices a nav item added without anyone deciding it belongs there.
  'Access tokens',
]

async function renderApp() {
  const view = render(<App />)
  await waitFor(() => screen.getAllByText('Test Portfolio'))
  return view
}

const collapse = async (user: ReturnType<typeof userEvent.setup>) => {
  await user.click(screen.getByRole('button', { name: 'Collapse navigation' }))
}

afterEach(() => {
  window.localStorage.clear()
  vi.restoreAllMocks()
})

describe('the nav rail toggle', () => {
  it('flips the container class and aria-expanded, and says which way it goes', async () => {
    const user = userEvent.setup()
    const { container } = await renderApp()
    const app = () => container.querySelector('.app-container')

    const toggle = screen.getByRole('button', { name: 'Collapse navigation' })
    expect(toggle).toHaveAttribute('aria-expanded', 'true')
    expect(app()).not.toHaveClass('app-container--nav-collapsed')

    await user.click(toggle)
    expect(app()).toHaveClass('app-container--nav-collapsed')
    expect(screen.getByRole('button', { name: 'Expand navigation' })).toHaveAttribute('aria-expanded', 'false')

    await user.click(screen.getByRole('button', { name: 'Expand navigation' }))
    expect(app()).not.toHaveClass('app-container--nav-collapsed')
    expect(screen.getByRole('button', { name: 'Collapse navigation' })).toHaveAttribute('aria-expanded', 'true')
  })

  it('keeps every nav item named and titled while the labels are hidden', async () => {
    const user = userEvent.setup()
    await renderApp()
    await collapse(user)

    for (const label of NAV_LABELS) {
      // The accessible name, not a class: the label text is still in the tree, which is
      // the whole difference between a clipped span and a `display: none` one.
      const item = screen.getByRole('button', { name: label })
      expect(item).toBeInTheDocument()
      expect(item).toHaveAttribute('title', label)
      expect(item).toHaveTextContent(label)
    }
  })

  it('reaches the nav items from the keyboard and activates them with Enter and Space', async () => {
    const user = userEvent.setup()
    await renderApp()

    await user.tab()
    expect(screen.getByRole('button', { name: 'Collapse navigation' })).toHaveFocus()
    await user.tab()
    expect(screen.getByRole('button', { name: 'Dashboard' })).toHaveFocus()
    await user.tab()
    expect(screen.getByRole('button', { name: 'Holdings & Tax lots' })).toHaveFocus()

    await user.keyboard('{Enter}')
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Holdings & Tax lots' })).toHaveClass('active'),
    )
    expect(screen.getByRole('button', { name: 'Dashboard' })).not.toHaveClass('active')

    const ledger = screen.getByRole('button', { name: 'Transaction Ledger' })
    ledger.focus()
    await user.keyboard(' ')
    await waitFor(() => screen.getAllByText('SPLIT'))
    expect(ledger).toHaveClass('active')
  })
})

describe('remembering the choice', () => {
  it('survives a remount', async () => {
    const user = userEvent.setup()
    const first = await renderApp()
    await collapse(user)
    expect(window.localStorage.getItem(KEY)).toBe('true')

    first.unmount()
    const second = render(<App />)
    await waitFor(() => screen.getAllByText('Test Portfolio'))
    expect(second.container.querySelector('.app-container')).toHaveClass('app-container--nav-collapsed')
    expect(screen.getByRole('button', { name: 'Expand navigation' })).toHaveAttribute('aria-expanded', 'false')
  })

  it('starts expanded when the stored value is not one it recognises', async () => {
    window.localStorage.setItem(KEY, 'not-a-boolean')
    const { container } = await renderApp()
    expect(container.querySelector('.app-container')).not.toHaveClass('app-container--nav-collapsed')
    expect(screen.getByRole('button', { name: 'Collapse navigation' })).toHaveAttribute('aria-expanded', 'true')
  })

  it('renders, expanded, when reading localStorage throws', async () => {
    // Safari private mode and a blocked third-party origin throw on access rather than
    // returning null, and that happens during the first render. A broken preference store
    // must cost the user the memory of the preference, not the page.
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('localStorage is blocked')
    })

    const { container } = render(<App />)
    await waitFor(() => screen.getAllByText('Test Portfolio'))
    expect(container.querySelector('.app-container')).not.toHaveClass('app-container--nav-collapsed')
    expect(screen.getByRole('button', { name: 'Collapse navigation' })).toHaveAttribute('aria-expanded', 'true')
  })

  it('still collapses for the session when writing localStorage throws, and remembers nothing', async () => {
    const user = userEvent.setup()
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('localStorage is full')
    })

    const { container } = await renderApp()
    await collapse(user)

    expect(container.querySelector('.app-container')).toHaveClass('app-container--nav-collapsed')
    expect(screen.getByRole('button', { name: 'Expand navigation' })).toBeInTheDocument()
    expect(window.localStorage.getItem(KEY)).toBeNull()
    await waitFor(() => screen.getAllByText('Test Portfolio'))
  })
})

describe('what the collapsed rail keeps', () => {
  it('keeps sign out, and names the portfolio everything on screen belongs to', async () => {
    const user = userEvent.setup()
    await renderApp()
    await collapse(user)

    expect(screen.getByRole('link', { name: 'Sign out' })).toBeInTheDocument()
    expect(
      screen.getByRole('button', { name: /Active portfolio: Test Portfolio/ }),
    ).toBeInTheDocument()
  })

  it('expands the rail and focuses the real select when that button is used', async () => {
    const user = userEvent.setup()
    const { container } = await renderApp()
    await collapse(user)

    await user.click(screen.getByRole('button', { name: /Active portfolio: Test Portfolio/ }))
    expect(container.querySelector('.app-container')).not.toHaveClass('app-container--nav-collapsed')
    expect(screen.getByRole('combobox')).toHaveFocus()
  })

  it('brings the creation actions back on the same click that expands the rail', async () => {
    const user = userEvent.setup()
    await renderApp()
    expect(screen.getByRole('button', { name: 'New Portfolio' })).toBeInTheDocument()

    await collapse(user)
    expect(screen.queryByRole('button', { name: 'New Portfolio' })).not.toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: 'Expand navigation' }))
    expect(screen.getByRole('button', { name: 'New Portfolio' })).toBeInTheDocument()
  })

  it('leaves nothing focusable behind that a keyboard user cannot see', async () => {
    const user = userEvent.setup()
    await renderApp()
    await collapse(user)

    const toggle = screen.getByRole('button', { name: 'Expand navigation' })
    // The one control a collapsed rail cannot afford to lose, checked on its own before the
    // walk: a real button, focusable, still named for what it will do.
    toggle.focus()
    expect(toggle).toHaveFocus()

    // Start the walk from the top of the document, not from the toggle the click above left
    // focused -- tabbing from a focused toggle correctly moves past it, and that would hide
    // the head of the order. The tab order is the assertion, not a stylesheet reading:
    // whatever the rail hides, it hides from the tab order as well.
    ;(document.activeElement as HTMLElement | null)?.blur()
    const stops: string[] = []
    // The accessible name, not the visible text: the toggle is an icon-only control whose
    // name comes from `aria-label`, so reading `textContent` alone would report it as ""
    // and claim a named control was unnamed. Same rule the rest of this file uses via
    // `getByRole({ name })`.
    const nameOf = (el: Element) => (el.getAttribute('aria-label') || el.textContent || '').trim()
    for (let i = 0; i < NAV_LABELS.length + 4; i++) {
      await user.tab()
      const el = document.activeElement
      if (!el || el === document.body || el === document.documentElement) break
      stops.push(nameOf(el))
    }

    expect(stops).toEqual([
      'Expand navigation',
      ...NAV_LABELS,
      'Active portfolio: Test Portfolio — expand to change',
      'Sign out',
    ])
  })
})
