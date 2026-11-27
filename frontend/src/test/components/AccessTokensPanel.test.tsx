import { describe, it, expect, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { http, HttpResponse } from 'msw';
import AccessTokensPanel from '../../components/AccessTokensPanel';
import { server } from '../mocks/server';

/**
 * The panel's two promises, each tested as the promise rather than as the pixels.
 *
 * Both are about what the panel does *not* do: it does not keep a secret around, and
 * it does not decide liveness for itself.
 */

const SECRET = 'pp_' + 'a'.repeat(43);

/** A token whose `expires_at` is in the future but which the server calls expired.
 * That combination is impossible in the database and deliberate here: it can only be
 * rendered "(expired)" by reading the server's `expired` field. A panel that compared
 * `expires_at` to its own clock — which is what it is tempting to write, and what the
 * React purity rule now refuses at lint — would show this row as live while the server
 * is refusing it at `lookup`. Same for `stale`. */
const EXPIRED_ROW = {
  id: 7,
  label: 'laptop agent',
  scopes: ['read'],
  created_at: '2026-01-01T00:00:00Z',
  expires_at: '2999-01-01T00:00:00Z',
  last_used_at: '2026-02-01T00:00:00Z',
  revoked_at: null,
  stale: false,
  expired: true,
};

/** And the mirror: idle long enough to nudge about, still live. */
const STALE_ROW = { ...EXPIRED_ROW, id: 8, label: 'old script', stale: true, expired: false };

function mockList(rows: unknown[] = [EXPIRED_ROW, STALE_ROW]) {
  server.use(http.get('/api/tokens', () => HttpResponse.json(rows)));
}

describe('AccessTokensPanel', () => {
  it('renders the server’s verdict on a token, not one it worked out itself', async () => {
    mockList();
    render(<AccessTokensPanel />);

    await screen.findByText('laptop agent');
    expect(screen.getByText(/\(expired\)/)).toBeInTheDocument();
    expect(screen.getByText(/unused for 90 days/)).toBeInTheDocument();
    // The nudge belongs to the live one; an expired token is past nudging about.
    expect(screen.getByText('old script').parentElement).toHaveTextContent('unused for 90 days');
    expect(screen.getByText('laptop agent').parentElement).not.toHaveTextContent('unused for 90 days');
  });

  it('shows the secret once, and it is gone when the dialog closes', async () => {
    mockList([]);
    server.use(
      http.post('/api/tokens', () =>
        HttpResponse.json({
          id: 9,
          secret: SECRET,
          label: 'phone',
          scopes: ['read', 'import'],
          expires_at: null,
          created_at: '2026-03-01T00:00:00Z',
        }),
      ),
    );

    render(<AccessTokensPanel />);
    await userEvent.type(screen.getByLabelText(/^Name/), 'phone');
    await userEvent.click(screen.getByRole('button', { name: /create token/i }));

    // Shown, and shown as the only showing.
    expect(await screen.findByText(SECRET)).toBeInTheDocument();
    expect(screen.getByText(/only time/i)).toBeInTheDocument();

    await userEvent.click(screen.getByRole('button', { name: /done/i }));
    await waitFor(() => expect(screen.queryByText(SECRET)).not.toBeInTheDocument());

    // And nothing in the list can bring it back: the list endpoint does not carry one,
    // so there is no reveal affordance to find.
    expect(screen.queryByText(/reveal|show secret/i)).not.toBeInTheDocument();
  });

  it('refuses to mint with no scope rather than sending a token that can do nothing', async () => {
    mockList([]);
    const post = vi.fn();
    server.use(
      http.post('/api/tokens', () => {
        post();
        return HttpResponse.json({});
      }),
    );

    render(<AccessTokensPanel />);
    await userEvent.type(screen.getByLabelText(/^Name/), 'oops');
    for (const box of await screen.findAllByRole('checkbox')) {
      if ((box as HTMLInputElement).checked) await userEvent.click(box);
    }

    expect(screen.getByRole('button', { name: /create token/i })).toBeDisabled();
    expect(post).not.toHaveBeenCalled();
  });
});
